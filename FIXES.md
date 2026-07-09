# Fix report — closing the review gaps against SPEC.md

Remediation plan for the findings in `SPEC-TUI.md` Appendix A (review of
2026-07-09). Each finding lists where the defect lives, the concrete fix,
and the test that proves it. §4 gives the implementation order — several
CLI fixes depend on small core additions, so the core lands first.

Severity legend: **[bug]** user-visible wrong behavior today,
**[gap]** specced behavior that is missing, **[hygiene]** internal quality,
**[decision]** code and spec disagree and one must be updated.

---

## 1. Correctness bugs

### F1 [bug] Export CSV is malformed and incomplete

**Where:** `crates/actng-cli/src/main.rs:332-371`.

Three defects in one block:

- Descriptions are interpolated into CSV lines with `format!` and no
  quoting (`main.rs:348`). Any description containing a comma — common in
  bank exports ("COOP, LAUSANNE") — shifts every following column and
  corrupts the file.
- Columns are `date,amount,description,tag`; the spec (`SPEC.md` §5.2)
  requires `date, description, amount, tag, category, source_file`.
- Only `result.tagged` is written. Entries in the review queue are
  silently dropped, violating the explicit rule that untagged entries
  export with an empty tag "so nothing silently disappears".

**Fix:** add an `export` module to `actng-core` (this is `SPEC-TUI.md`
§5.4, Phase T0 task 3):

```rust
pub fn write_csv(w: impl Write, dataset: &Dataset, profile: &Profile,
                 suggestions: &[Option<Suggestion>]) -> Result<Summary, Error>;
pub struct Summary { pub rows: usize, pub per_category: Vec<(String, f64)> }
```

Use the `csv` crate (already a core dependency) for RFC-4180 quoting. Emit
one row per dataset entry — tagged rows carry tag + category (empty
category if unset), untagged rows carry empty tag and category. `Summary`
carries the per-category totals the CLI prints with `--summary`. Rewrite
the CLI `Export` arm to: `collect` → `suggest` per entry → `write_csv` to
the output file. Depends on F12's `Dataset` (source-file tracking).

**Tests:** core unit tests — a description containing `,` and `"`
round-trips through a `csv::Reader`; untagged entries appear with empty
tag; per-category totals match hand-computed sums. CLI integration test
(F11) asserts column headers and row count = dataset size.

### F2 [bug] `ACTNG_PROFILE` overrides an explicit `--profile` flag

**Where:** `crates/actng-cli/src/main.rs:102-106` — `resolve_profile_path`
checks the env var first and ignores the flag whenever the var is set.
Because the flag has `default_value = "actng.json"` (`main.rs:13`), the
code can't distinguish "user passed `--profile x.json`" from "default".

**Fix:** make the flag `Option<PathBuf>` with no default, then resolve
flag > env > `./actng.json`:

```rust
#[arg(short, long)]
profile: Option<PathBuf>,

fn resolve_profile_path(flag: Option<&Path>) -> PathBuf {
    flag.map(Path::to_path_buf)
        .or_else(|| std::env::var_os("ACTNG_PROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("actng.json"))
}
```

**Tests:** unit test on `resolve_profile_path` covering all three arms;
integration test (F11) running the binary with both flag and env set,
asserting the flag wins.

### F3 [bug] Detected layouts are never persisted — fingerprint reuse is dead code

**Where:** no caller. `read_entries_from_path_with_layouts`
(`crates/actng-core/src/import.rs:456`) reads `profile.layouts`, but
nothing ever inserts into it, so the map is empty forever and every import
re-runs auto-detection — the exact failure mode `SPEC.md` §4.1 exists to
prevent.

**Fix:** add the helper from `SPEC-TUI.md` §5.5:

```rust
impl Profile {
    /// Insert fingerprint→layout for every successful import not already
    /// remembered. Returns how many were added.
    pub fn remember_layouts(&mut self, imports: &[FileImport]) -> usize;
}
```

Call it (and save the profile when it returns > 0) in the CLI `scan`,
`tag`, `review`, and `export` arms right after `import_dir`. `Import`
already exposes `fingerprint` and `profile` (`import.rs:35-39`), so this
is a straightforward map insert with an `entry().or_insert` guard —
remembered layouts must not be overwritten by re-detection.

**Tests:** core unit test — import a fixture twice; after
`remember_layouts` the second `import_dir` gets a fingerprint hit (assert
by mutating the stored layout and observing it take effect). Idempotence:
calling twice returns 0 the second time.

### F4 [bug] Exit code 2 for "entries remain untagged" is unimplemented

**Where:** `crates/actng-cli/src/main.rs:108` — `main` returns
`anyhow::Result<()>`, so the process only ever exits 0 or 1. `SPEC.md`
§5.1 requires exit 2 after `actng tag` when the review queue is non-empty,
so scripts can detect incomplete state.

**Fix:** change `main` to return `std::process::ExitCode`; wrap the
current body in a `run() -> anyhow::Result<ExitCode>` that returns
`ExitCode::from(2)` from the `Tag` arm when `!result.review.is_empty()`,
`ExitCode::SUCCESS` otherwise; map `Err` to exit 1 with the error printed
to stderr (not stdout, so `--format csv|json` output stays pipeable).

**Tests:** integration test (F11): `tag` on an untrained profile exits 2;
after scripted review of everything, exits 0.

### F5 [bug] Review progress counter shows the wrong denominator

**Where:** `crates/actng-cli/src/main.rs:222` — the loop prints
`[{i+1}/{total_entries}]` while iterating `queue`, so without `--all` a
40-entry queue over a 300-entry dataset ends at `[40/300]`.

**Fix:** capture `queue.len()` before the loop and print that. One line,
fold into the F10 rewrite.

---

## 2. Spec gaps (missing behavior)

### F6 [gap] `tag` has no summary line

**Where:** `crates/actng-cli/src/main.rs:161-193`.

**Fix:** after printing results in `table` format, print to stderr for
`csv`/`json` (keeps stdout machine-readable), stdout for `table`:

```
312 tagged (287 exact, 25 bayes), 40 need review, 12 duplicates skipped
```

Counts come from `result.tagged` partitioned on `Suggestion.source`
(`Source::Exact` / `Source::Bayes`), `result.review.len()`, and
`result.duplicates_dropped`. Lands together with F4 (same arm).

**Tests:** integration test asserts the summary format against a fixture
dir with a known-trained profile.

### F7 [gap] `scan` reports none of the specced detail

**Where:** CLI `crates/actng-cli/src/main.rs:139-160` prints only entry
counts. Root cause is partly in core: the detected delimiter and the
encoding fallback are computed inside `import.rs` (`sniff_delimiter`
`import.rs:275`, `decode_windows1252` `import.rs:259`) but never surfaced
on `Import`, so no frontend *can* report them.

**Fix (core):** extend `Import` with detection metadata:

```rust
pub struct Import {
    pub profile: ImportProfile,
    pub entries: Vec<Entry>,
    pub fingerprint: String,
    pub delimiter: u8,
    pub encoding: Encoding,        // enum { Utf8, Windows1252 }
    pub skipped_rows: usize,       // preamble + rows that failed to parse
}
```

(Additive fields; `Import` is not serialized, so no format concern.)

**Fix (CLI):** rewrite the `Scan` arm to print per file: delimiter,
encoding, header/headerless, date-format list from `ImportProfile`,
entry count, date range (min/max over `entry.date`), skipped-row count,
and layout provenance — `remembered` when the fingerprint was already in
`profile.layouts` before this run, else `detected`. Errors keep the
current inline treatment. `scan` stays read-only *except* for F3's
`remember_layouts` persistence, which `SPEC.md` task 12 expects here.

**Tests:** core unit tests that a `;`-delimited windows-1252 fixture
reports `delimiter: b';'`, `Encoding::Windows1252`; integration test
snapshots the scan table for a small fixture set.

### F8 [gap] `tags list` lacks trained-doc counts — core has no accessor

**Where:** CLI `crates/actng-cli/src/main.rs:319-330`; core gap in
`crates/actng-core/src/tagger.rs` (both `Tagger.exact` and
`NaiveBayes.doc_counts` are private, `bayes.rs:13`).

**Fix (core):** `SPEC-TUI.md` §5.2 — `Tagger::stats() -> Vec<TagStats>`
with `trained_docs` (from `doc_counts`) and `exact_keys` (count of exact
values per tag), sorted by tag name. Add
`NaiveBayes::doc_count(&self, tag) -> u64` to keep the field private.

**Fix (CLI):** `tags list` prints `tag / category / trained docs / exact
keys`, joining declared tags with stats (a declared-but-untrained tag
shows 0s; a trained-but-undeclared tag can't exist — `learn`
auto-registers).

**Tests:** core unit test — learn 3 descriptions under one tag, 1 under
another; assert stats. CLI integration test covers the table.

### F9 [gap] `tags rm` deletes training data without confirmation

**Where:** `crates/actng-cli/src/main.rs:304-310`.

**Fix:** before removing, load stats (F8) and prompt:
`Delete 'climbing' and its 7 trained documents? [y/N]` via
`dialoguer::Confirm` (see F10 — the dependency is already declared).
Add `--yes` to skip the prompt for scripting. Abort without saving on
anything but yes.

**Tests:** integration test piping `n` (profile unchanged) and using
`--yes` (tag gone, suggestions gone).

### F10 [gap] `review` doesn't match the specced interaction and has no quit

**Where:** `crates/actng-cli/src/main.rs:194-295`. Hand-rolled stdin
parsing; `dialoguer` is declared in `crates/actng-cli/Cargo.toml` but
never used; there is no way to stop mid-session except Ctrl-C (safe, but
specced as an explicit option).

**Fix:** rebuild the loop on `dialoguer::Select` per `SPEC.md` §5.2: the
item list is the ranked candidates (with confidence), then `new tag…`,
`skip`, `quit`. `new tag…` chains a `dialoguer::Input` (this replaces the
current prefix-matching — a nice idea, but ambiguity handling and
new-tag-on-typo footguns go away with an explicit select). Keep: learn +
atomic save after every confirmation, `--all` retag mode, quick-select by
number (Select gives arrow keys + typed index). `quit` exits the loop,
prints how many remain. Fold in F5's counter fix. Drop the redundant
final save (`main.rs:290` — every answer already saved).

**Tests:** dialoguer prompts don't drive well under `assert_cmd`; keep
the loop thin and test the decision logic (candidate ordering, learn
side-effects) via core tests, plus one integration test that runs
`review` on an empty queue and asserts the "no entries" path. Manual
checklist for the interactive path.

### F11 [gap] No CLI integration tests

**Where:** `crates/actng-cli` has zero tests; `SPEC.md` task 17.

**Fix:** add `assert_cmd` + `predicates` as dev-dependencies and
`crates/actng-cli/tests/cli.rs` running the binary in a `tempfile` dir
seeded with fixture CSVs:

1. `init --name test --tags groceries,rent` → profile file exists, re-init
   fails.
2. `scan` → table lists the fixtures (F7 format).
3. `tag` → exit code 2, summary line (F4, F6).
4. Simulate training by calling core `Profile::learn` + save directly
   (avoids scripting dialoguer), then `tag` → exit 0.
5. `tags list` / `tags rm --yes` (F8, F9).
6. `export -o out.csv --summary` → parse `out.csv` with the `csv` crate,
   assert columns and untagged rows (F1).
7. Flag-vs-env precedence (F2).

Every fix above lands with its integration case, so this file grows with
§4's order rather than arriving at the end.

---

## 3. Hygiene and decisions

### F12 [hygiene] `Profile::run` duplicates the dedup logic

**Where:** `crates/actng-core/src/profile.rs:155-173` re-implements
`discover::deduplicate` (`discover.rs:62-83`) inline, on references
instead of owned entries — the same `(date, key, cents)` fingerprint in
two places that can drift.

**Fix:** implement `Dataset` + `collect` (`SPEC-TUI.md` §5.3): owned
deduplicated entries, per-entry source index, per-file failures,
duplicates-dropped count. Reimplement `Profile::run` on top of it and
delete both the inline copy and the now-unused `deduplicate` (or keep
`deduplicate` as a thin wrapper if external callers want it — nothing
uses it today outside tests). `Dataset` is also the prerequisite for F1
(`source_file` column) and the TUI.

**Tests:** existing `profile_run_*` tests in `tests/postfinance.rs` must
pass unchanged; new unit test that `collect` reports source indices and
failures for a dir mixing good and broken files.

### F13 [decision] `discover` disagrees with SPEC.md §4.3

**Where:** `crates/actng-core/src/discover.rs:16-43` — recursive descent,
and `.txt`/`.tsv` accepted by extension alone; spec says non-recursive v1
and `.txt` "only if the content sniffs as delimited".

**Recommendation:** split the difference, updating both code and spec:

- **Keep recursion** and amend `SPEC.md` §4.3 — it's implemented, tested
  (`discover_finds_bank_files_recursively`), and matches how people
  organize exports (per-year subfolders). Skip hidden directories
  (`.git`, `.cache`) while descending.
- **Implement `.txt` sniffing** as specced: read the first ~5 lines and
  accept only if `sniff_delimiter`'s best score clears a consistency
  threshold. The current behavior imports any stray `notes.txt` and then
  reports it as a per-file error at best, noise at worst. `.tsv` by
  extension is fine to keep (it declares its format).

Requires updating `discover_finds_bank_files_recursively`
(`discover.rs:93-114`), which currently asserts that a `notes.txt`
containing prose *is* discovered.

**Tests:** prose `.txt` excluded; tab/`;`-delimited `.txt` included;
hidden dirs skipped.

---

## 4. Implementation order

Ordered so every step compiles, is tested, and ships value alone.
Steps 1–4 are the core work; they are the same items as `SPEC-TUI.md`
Phase T0, so this plan and the TUI plan converge.

| # | Fixes | Scope |
|---|-------|-------|
| 1 | F8 core (`Tagger::stats`) + F3 core (`remember_layouts`) | small, independent |
| 2 | F12 (`Dataset` + `collect`, `run` on top) | core refactor |
| 3 | F1 core (`export::write_csv` + `Summary`) | needs 2 |
| 4 | F7 core (`Import` detection metadata) | additive |
| 5 | F2 (flag/env precedence) + F4 (exit codes) + F6 (summary line) | CLI, one PR |
| 6 | F11 harness + integration cases for 1–5 | dev-deps, tests |
| 7 | F1 CLI (`export` on shared writer) + F3 CLI (persist layouts in scan/tag/review/export) | CLI |
| 8 | F7 CLI (`scan` detail) + F8 CLI (`tags list` counts) | CLI |
| 9 | F9 (`tags rm` confirm) + F10 (`review` on dialoguer, F5 counter) | CLI |
| 10 | F13 (spec amendment + `.txt` sniffing) | core + `SPEC.md` edit |

**Definition of done:** `cargo test --workspace` green including the new
`tests/cli.rs`; `cargo clippy` clean; `SPEC.md` §4.3 amended per F13;
manual smoke of the §5.3 canonical session in a scratch folder confirms —
layouts persisted after first `scan` (profile JSON gains a `layouts`
entry), `tag` exits 2 then 0 after review, export opens correctly in a
spreadsheet with quoted descriptions.
