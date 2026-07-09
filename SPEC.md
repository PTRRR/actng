# actng — specification

A tool that reads bank-export CSV files, tags each entry (transaction), and
groups tags into categories. Tagging is learned from the user: they confirm or
correct tags during review, and the tool gets better with every correction.

Status legend used throughout: **[done]** already implemented,
**[partial]** exists but needs hardening, **[todo]** not started.

---

## 1. Goals and non-goals

### Goals

- **Format-flexible import.** Bank CSVs differ per bank, per country, per
  export button. The importer auto-detects layout (delimiter, header, column
  roles, date format, encoding) and must survive real-world quirks. A layout
  can also be specified explicitly and reused.
- **Learned tagging, not rule maintenance.** No regex rulebooks. A
  three-stage pipeline: noise normalization → exact-match memory →
  Naive Bayes fallback with abstention. Below-threshold entries go to a
  review queue instead of being guessed.
- **Portable profiles.** Everything the user has taught the tool — the tag
  set, the trained weights, the exact-match memory, remembered file layouts —
  lives in one JSON file (the *profile*) that can be copied to another
  machine, versioned, shared, or kept per-context (personal vs. business).
- **Folder-oriented CLI.** `actng` is run inside a folder of bank exports. It
  discovers the files, imports everything, applies the profile, and walks the
  user through whatever it couldn't tag confidently.
- **GUI-ready core.** All business logic lives in the `actng-core` library.
  The core never touches stdin/stdout/terminal, holds all state in
  serializable types, and exposes step-wise operations (suggest → confirm →
  learn) so an interactive GUI can drive the same engine later.

### Non-goals (v1)

- No network access, no online model, no external ML dependencies.
- No database — state is the profile file; tagged output is regenerated on
  demand (see §4.4).
- No bank-API/OFX/MT940/camt.053 ingestion (CSV only; the `Entry` model
  doesn't preclude adding readers later).
- No budgeting/analytics beyond a per-tag/per-category summary export.

---

## 2. Architecture

```
actng/                      Cargo workspace
├── crates/
│   ├── actng-core/         library: all business logic          [partial]
│   └── actng-cli/          binary: user-facing CLI              [todo]
├── examples/               real anonym-ish exports for tests
└── testdata/bank2ynab/     30 vendored fixtures (MIT)           [done]
```

**Dependency rule:** `actng-cli` → `actng-core`. Never the other way. A
future `actng-gui` would sit next to `actng-cli` and depend only on the core.

**Core purity rule:** `actng-core` does no terminal I/O and makes no
decisions that belong to a frontend (what to display, when to prompt, when to
save). It reads/writes files only through explicit functions the caller
invokes (`read_entries_from_path`, `Profile::load/save`, `discover`).

### 2.1 actng-core module map

| Module         | Responsibility                                              | Status    |
|----------------|-------------------------------------------------------------|-----------|
| `entry`        | `Entry { date, description, amount, raw }`                  | done      |
| `error`        | `Error` enum (thiserror)                                    | done      |
| `import`       | CSV reading, `ImportProfile` auto-detection                 | partial   |
| `normalize`    | noise filtering, tokenization, stable entry key             | done      |
| `bayes`        | incremental Naive Bayes over token sets                     | done      |
| `tagger`       | exact-memory + Bayes engine, `Suggestion`, learn/suggest    | done      |
| `profile`      | `Profile`: tags, categories, tagger, layouts, save/load     | todo      |
| `discover`     | find bank files in a folder, batch import, dedup            | todo      |

---

## 3. The tagging pipeline (implemented — reference)

For every entry description:

1. **Normalize** (`normalize.rs`): Unicode-fold (NFKD, strip diacritics,
   lowercase); extract IBANs and phone numbers as counterparty features;
   strip dates, card masks (`XXXX0918`), decimal amounts, mostly-numeric
   tokens, and multilingual stopwords. Produces `tokens` plus a `key` — the
   sorted, deduplicated token join — that is stable across dates and card
   numbers for the same merchant.
2. **Exact match**: if `key` was tagged before, return that tag with
   confidence 1.0. Deterministic; the same merchant never asks twice.
3. **Naive Bayes fallback**: set-of-words multinomial NB with Laplace
   smoothing and log-sum-exp posteriors. Unknown tokens are ignored; with no
   known token there is *no* guess. The top posterior is returned only if it
   clears `min_confidence` (default 0.8); otherwise the entry goes to review.
4. **Learn**: a confirmed tag updates both the exact memory (insert/overwrite
   key) and the Bayes counts. Training is incremental — no retraining pass.

Design consequence: **entry→tag assignments are not stored separately.**
Re-running the tagger over the same files reproduces every confirmed tag via
the exact-match layer. The profile *is* the state. (Caveat: two entries with
the same normalized key can only carry one tag — last confirmation wins.
Acceptable for v1; revisit if amount-dependent tagging is ever needed.)

---

## 4. Core additions

### 4.1 `Profile` (module `profile`) — the central user artifact

```rust
pub struct Tag {
    pub name: String,               // "groceries"
    pub category: Option<String>,   // "living" — categories are derived from tags
    pub description: Option<String>,
}

pub struct Profile {
    pub version: u32,               // format version, currently 1
    pub name: String,               // "personal", "acme-sarl", ...
    pub tags: Vec<Tag>,             // the declared tag set
    pub tagger: Tagger,             // exact memory + Bayes weights + min_confidence
    /// Remembered CSV layouts keyed by file fingerprint, so a re-export from
    /// the same bank never re-runs (or mis-runs) auto-detection.
    pub layouts: HashMap<String, ImportProfile>,
}
```

Behavior:

- `Profile::new(name)` — empty profile, default tagger.
- `Profile::load(path)` / `save(path)` — pretty JSON. `load` rejects
  `version > CURRENT` with a clear error; older versions get migrated in
  code when the format ever changes.
- `profile.learn(description, tag)` — delegates to `Tagger::learn` and
  auto-registers the tag in `tags` if it's new (category left `None` until
  the user sets it).
- `profile.suggest / candidates` — delegate to the tagger.
- Tag management: `add_tag`, `rename_tag` (must rewrite exact-memory values
  and Bayes tag keys), `set_category`, `remove_tag` (drops training data for
  that tag).
- **Layout fingerprint:** for files with a header row, the fingerprint is the
  lowercased, joined header; for headerless files, the delimiter + column
  count + per-column detected kind. On import, a fingerprint hit uses the
  stored `ImportProfile`; a miss runs detection and the caller may persist
  the result.

Rationale for one file instead of separate tags/weights files: the weights
are meaningless without the tag set and vice versa; one artifact is what
makes "copy it to another machine and keep training there" trivially safe.

### 4.2 Importer hardening (module `import`)

Driven by the vendored `testdata/bank2ynab/` fixtures. Each item below has a
concrete fixture that currently fails or would fail:

- **Encoding fallback:** input that isn't valid UTF-8 is decoded as
  Windows-1252 (superset of latin-1 for the bytes that occur in practice);
  implemented as a small manual byte→char mapping, no new dependency.
  *(iso-8859-1 and cp1250 fixtures.)*
- **Delimiter sniffing:** try `,` `;` `\t` `|`; score each by per-line field
  count consistency over the first ~20 lines and pick the winner.
  *(Polish/Czech/Belgian `;` files, `test_delimiter_tab.csv`.)*
- **Preamble skipping:** drop leading records whose field count differs from
  the modal field count of the file before header detection.
  *(`TransactionHistory_20180418043121.csv` metadata rows.)*
- **Split debit/credit columns:** `ImportProfile` gains
  `debit_column: Option<usize>` and `credit_column: Option<usize>` (header
  hints: debit/debet/outflow/kiadás; credit/kredit/inflow/bevétel); effective
  amount = credit − debit. *(BOI, KBC, Hungarian EBH fixtures.)*
- **Currency-symbol-tolerant amounts:** `parse_amount` strips a leading/
  trailing currency symbol (£ € $ CHF) but only when the remainder is fully
  numeric — `"22FEB"` must stay non-numeric. *(`MS_JANE_SMITH_*.csv`.)*
- **More date formats:** `%d %b %Y`, `%d %b %y` ("10 Feb 18"), `%Y.%m.%d.`
  (Hungarian trailing dot). Non-date noise in the date column ("Pending")
  parses to `date: None` without failing the row.

### 4.3 Discovery and batch import (module `discover`)

```rust
/// CSV-ish files under `dir`, found recursively (hidden directories such as
/// `.git` are skipped), sorted by name.
pub fn discover(dir: &Path) -> Result<Vec<PathBuf>, Error>;

pub struct FileImport { pub path: PathBuf, pub result: Result<Import, Error> }

/// Import every discovered file; per-file failures are reported, not fatal.
/// `profile.layouts` provides fingerprint→ImportProfile reuse (see §4.1).
pub fn import_dir(dir: &Path, profile: &Profile) -> Result<Vec<FileImport>, Error>;

/// Deduplicate entries across files and produce a `Dataset` (§4.4) — owned
/// entries, per-entry source file, per-file failures, duplicates-dropped
/// count. The `RunResult`-borrowing shape of `Profile::run` (§4.4) is built
/// on top of this.
pub fn collect(imports: Vec<FileImport>) -> Dataset;
```

- **Recursive by default.** Bank exports are commonly organized in
  per-year or per-account subfolders; discovery descends into them,
  skipping directories whose name starts with `.`.
- Discovery matches extensions case-insensitively: `.csv` and `.tsv` by
  extension alone; `.txt` only if the content sniffs as a delimited table
  (at least two consistently-split columns across sampled lines) rather
  than free-form prose — a stray `notes.txt` in the folder is not silently
  imported and reported as a parse failure.
- **Deduplication:** overlapping exports are common (user downloads
  Jan–Mar and Feb–Apr). Entry fingerprint = `(date, normalized key, amount)`;
  duplicates across files are collapsed, keeping the first occurrence and
  counting the drops so the CLI can report them.

### 4.4 Applying a profile (batch result type)

```rust
pub struct Dataset {
    pub entries: Vec<Entry>,          // deduplicated, in file order
    pub source: Vec<usize>,           // parallel: index into `sources`
    pub sources: Vec<PathBuf>,        // files that contributed at least one entry
    pub failures: Vec<(PathBuf, Error)>,
    pub duplicates_dropped: usize,
}

pub struct Review<'a> { pub entry: &'a Entry, pub candidates: Vec<(String, f64)> }

pub struct RunResult<'a> {
    pub tagged:  Vec<(&'a Entry, Suggestion)>, // confidence >= threshold
    pub review:  Vec<Review<'a>>,              // needs a human
    pub sources: Vec<PathBuf>,                 // files that contributed
    pub duplicates_dropped: usize,
}

impl Profile {
    pub fn run<'a>(&self, dataset: &'a Dataset) -> RunResult<'a>;

    /// Insert fingerprint→layout for every successful import not already
    /// remembered. Returns how many were newly persisted; the caller saves
    /// the profile when this is > 0.
    pub fn remember_layouts(&mut self, imports: &[FileImport]) -> usize;
}
```

`Dataset` (built by `discover::collect`, §4.3) owns its entries — unlike a
borrowed `&[FileImport]`, it can be held onto and re-queried after the
originating imports are consumed, which both the CLI export path and an
interactive frontend need. `Profile::run` is the single call a CLI *or* GUI
makes after import: everything confidently tagged on one side, everything
needing attention (with ranked candidates pre-computed for the picker UI) on
the other.

### 4.5 Tagger statistics and CSV export

```rust
pub struct TagStats { pub tag: String, pub trained_docs: u64, pub exact_keys: usize }
impl Tagger { pub fn stats(&self) -> Vec<TagStats>; }  // sorted by tag

// module `export`
pub struct Summary { pub rows: usize, pub per_category: Vec<(String, f64)> }
pub fn write_csv(w: impl Write, dataset: &Dataset, profile: &Profile,
                 suggestions: &[Option<Suggestion>]) -> Result<Summary, Error>;
```

`write_csv` is the shared implementation behind `actng export` (§5.2):
`date, description, amount, tag, category, source_file`, RFC-4180 quoted,
one row per dataset entry — tagged or not, so nothing silently disappears.
`Summary.per_category` buckets every row into its tag's category, or
`"uncategorized"` if it has none, so the totals always sum to the dataset.

---

## 5. The CLI (`crates/actng-cli`, binary name `actng`)

Dependencies: `clap` (derive), `anyhow`, `dialoguer` (review prompts),
`serde_json`. Nothing else unless forced.

### 5.1 Global behavior

- `--profile <path>` on every command; default `./actng.json`, overridable
  via `ACTNG_PROFILE` env var. Commands that mutate the profile save it back
  atomically (write temp file, rename).
- `[dir]` positional argument where relevant; default `.`.
- Exit codes: 0 ok, 1 error, 2 = "entries remain untagged" (so scripts can
  detect an incomplete state after `actng tag`).

### 5.2 Commands

```
actng init [--name <name>]         create a new profile file
actng scan [dir]                   discover + parse, report only (read-only)
actng tag  [dir] [--format ...]    apply profile, output tagged entries
actng review [dir]                 interactively tag what the model can't
actng tags [add|rm|category|list]  manage the tag set
actng export [dir] -o out.csv      tagged dataset + per-category summary
actng profile info                 show profile stats
```

- **`init`** — refuses to overwrite an existing profile. Optionally seeds
  tags: `actng init --tags groceries,rent,salary`.
- **`scan`** — per file: detected delimiter/encoding/layout, entry count,
  date range, parse failures. This is the debugging surface for the importer;
  it never touches the profile.
- **`tag`** — runs §4.4 and prints results. `--format table|csv|json`
  (default `table`). Summary line: `312 tagged (287 exact, 25 bayes),
  40 need review, 12 duplicates skipped`. Exit code 2 when review is
  non-empty.
- **`review`** — the training loop. For each review entry, in file order:
  show date/amount/description, show ranked candidates as a `dialoguer`
  select (plus "new tag…", "skip", "quit"). Every confirmation calls
  `profile.learn` immediately; the profile is saved after each answer
  (cheap, and a Ctrl-C loses nothing). A retag mode
  (`review --all`) walks *every* entry, including already-tagged ones, for
  correcting mistakes.
- **`tags`** — `list` shows tag → category → trained-doc count;
  `category <tag> <category>` assigns; `rm` warns that training data for the
  tag is dropped and asks for confirmation.
- **`export`** — the end product: one CSV of
  `date, description, amount, tag, category, source_file` plus (with
  `--summary`) per-category totals. Untagged entries export with an empty
  tag so nothing silently disappears.

### 5.3 User flow (canonical session)

```
$ cd ~/bank-exports
$ actng init --name personal
$ actng tag                # first run: everything lands in review
$ actng review             # user tags a handful of entries per merchant
$ actng tag                # exact + bayes now cover most of the file
$ actng review             # mop up the tail
$ actng export -o 2026.csv --summary
# later, on another machine:
$ scp actng.json elsewhere: && actng tag --profile actng.json
```

---

## 6. Testing strategy

- **Unit tests** per module (existing: 13, keep growing with each feature).
- **Fixture suite** `tests/fixtures.rs`: data-driven over
  `testdata/bank2ynab/*.csv` with a per-file expectations table (min entries,
  dates parse, amounts parse, description non-empty). Every §4.2 hardening
  item lands together with the fixture that proves it.
- **End-to-end test** against the real PostFinance export (existing,
  `tests/postfinance.rs`): import → learn 4 entries → exact recall, Bayes
  generalization, abstention, file-wide coverage.
- **Accuracy benchmark**: generate labeled transactions locally with the
  open-source Sparkov generator (Kaggle data is license-unclear; the
  BankTextCategorizer/banking-class repos have no license and must not be
  vendored). Train/test split, assert a floor on Bayes accuracy and on
  abstention correctness so classifier regressions are caught.
- **CLI tests**: `assert_cmd`-style integration tests running the binary in a
  temp dir with fixture files — init → tag → (scripted) review → export.

---

## 7. Ordered implementation tasks

Each task compiles, passes tests, and is independently useful before the next
one starts. Dependencies are strictly earlier-numbered.

**Phase A — make the core robust (current phase)**

1. **[in progress] Fixture test harness.** `tests/fixtures.rs` iterating over
   `testdata/bank2ynab/`, expectations table per file, currently-failing
   fixtures marked as known-failing so the suite is green and each hardening
   step flips expectations on.
2. **Encoding fallback + delimiter sniffing** (§4.2). Flips the iso-8859-1,
   cp1250, `;` and tab fixtures to passing.
3. **Preamble skipping + split debit/credit columns** (§4.2). Flips the
   TransactionHistory, BOI, KBC, EBH fixtures.
4. **Amount & date format tolerance** (currency symbols, `%d %b %Y`,
   `%Y.%m.%d.`, "Pending" tolerance). Flips the remaining fixtures; the
   known-failing list is now empty and stays empty.
5. **Tagger accuracy benchmark** with locally generated Sparkov data (§6).

**Phase B — profile and folder pipeline (core)**

6. **`Profile` type** (§4.1): `Tag`, versioned JSON save/load, learn/suggest
   delegation, tag auto-registration. Round-trip and version-rejection tests.
7. **Tag management ops**: `rename_tag` (rewrites tagger state), `set_category`,
   `remove_tag`. Tests that renames preserve suggestions.
8. **Layout memory**: file fingerprinting, `layouts` map wired into import.
9. **`discover` + `import_dir`** (§4.3) with per-file error capture and
   cross-file dedup. Tests over a temp dir mixing fixtures and junk files.
10. **`Profile::run` → `RunResult`** (§4.4). Test partitioning tagged/review
    on the PostFinance file.

**Phase C — CLI**

11. **Scaffold `actng-cli`**: clap skeleton with all commands stubbed,
    `--profile` resolution, atomic profile save helper, error → exit-code
    mapping. `actng init` and `actng profile info` fully working.
12. **`actng scan`**: read-only report per file. This exercises tasks 8–9 in
    the wild.
13. **`actng tag`** with `--format table|csv|json` and exit code 2 semantics.
14. **`actng tags`** subcommands.
15. **`actng review`**: dialoguer loop, learn + save per answer, `--all`
    retag mode.
16. **`actng export`** with `--summary`.
17. **CLI integration tests** (temp-dir end-to-end, §6).

**Phase D — polish**

18. **Docs pass**: README with the §5.3 session, crate-level docs, `cargo
    doc` clean, clippy/fmt clean.
19. *(Deferred ideas, explicitly out of v1: recursive discovery, XLSX/OFX
    readers, amount-aware tagging, multi-tag entries, GUI crate.)*
