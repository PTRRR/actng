# actng-tui — complementary specification

Companion to `SPEC.md`. A full-screen terminal UI (`actng-tui`) that drives
the same `actng-core` engine as the CLI: manage profiles graphically, train
the tagger through a fast review loop, browse and retag entries, manage the
tag set, and export — without leaving one keyboard-driven interface.

The CLI stays the scriptable surface; the TUI is the interactive one. Both
read and write the *same* profile JSON and produce the *same* export CSV, so
they are interchangeable mid-session.

---

## 1. Goals and non-goals

### Goals

- **The whole canonical session in one screen set.** Everything §5.3 of
  `SPEC.md` does across five CLI invocations (scan → tag → review → review →
  export) is reachable inside the TUI, with live counts updating as the user
  trains.
- **Training as the centerpiece.** The review loop is where users spend
  their time; it must be fast: number keys confirm ranked candidates, a
  fuzzy picker covers the rest, new tags are created inline, one keystroke
  undoes a mistake, and confirming a merchant auto-resolves every other
  queued entry with the same normalized key.
- **Full profile management.** Add/rename/delete tags, assign categories,
  see per-tag training volume — the operations `Profile` already exposes,
  surfaced with confirmation where data is destroyed.
- **Crash-safe by construction.** The profile is saved atomically after
  every mutation (same guarantee as the CLI review loop); a Ctrl-C or
  terminal kill loses nothing.
- **Core purity preserved.** The TUI is a frontend like any other: it calls
  the same explicit core functions, holds all domain state in core types,
  and adds no business logic. Anything the TUI needs that smells like
  business logic (undo, stats, shared export writing) is added to
  `actng-core` first (§5) so the CLI benefits equally.

### Non-goals (v1)

- No charts or analytics beyond counts and per-category totals.
- No editing of imported entries (dates, amounts, descriptions are
  read-only facts from the bank).
- No async runtime or background threads — the dataset is thousands of
  rows; recomputing every suggestion synchronously is instant.
- No mouse-first flows (basic mouse click/scroll may work where ratatui
  gives it for free; never required).
- No theming/config file; colors respect the terminal palette.
- No multi-profile switcher — open a different profile via `--profile`,
  as with the CLI.

---

## 2. Architecture

```
actng/
├── crates/
│   ├── actng-core/         library: all business logic
│   ├── actng-cli/          binary: scriptable CLI
│   └── actng-tui/          binary: full-screen TUI        [this spec]
```

**Dependency rule:** `actng-tui` → `actng-core` only. Never `actng-cli`
(shared behavior belongs in the core, not in a sibling binary).

Dependencies: `ratatui`, `crossterm`, `anyhow`. Nothing else unless forced —
fuzzy matching is a ~30-line case-insensitive subsequence scorer, not a
crate.

### 2.1 Elm-style state machine

The binary is a classic Model/Message/Update/View loop so that all logic is
testable without a terminal:

```rust
struct App {
    profile: Profile,
    profile_path: PathBuf,
    dir: PathBuf,
    dataset: Dataset,                     // owned entries + sources (§5.3)
    suggestions: Vec<Option<Suggestion>>, // parallel to dataset.entries
    screen: Screen,                       // Overview | Review | Entries | Tags | Files
    modal: Option<Modal>,                 // picker, confirm, export form, help, error
    undo: Vec<UndoRecord>,                // session-scoped (§4.2)
    toast: Option<Toast>,
    last_saved: Option<Instant>,
}

enum Msg { Key(KeyEvent), Resize(u16, u16), Tick }

/// Pure except for the commands it *returns*; the caller performs them.
fn update(app: &mut App, msg: Msg) -> Vec<Cmd>;
enum Cmd { SaveProfile, WriteExport(PathBuf, bool), Quit }

fn view(app: &App, frame: &mut Frame);   // no mutation, no I/O
```

**Recompute-everything strategy:** after every `learn`, `unlearn`,
`rename_tag`, or `remove_tag`, the TUI re-runs `suggest` over all entries.
This is O(n) hash lookups plus Naive Bayes over a few thousand rows —
milliseconds — and buys total consistency: no cache invalidation, and the
Review queue automatically shrinks when one confirmation resolves sibling
entries (§4.2).

### 2.2 Terminal lifecycle

Raw mode + alternate screen on start; both restored on exit *and* in a
panic hook installed before ratatui init, so a bug never leaves the user's
shell corrupted. Terminals smaller than 80×24 render a single "terminal too
small" message instead of a broken layout.

---

## 3. Launch and global behavior

```
actng-tui [dir] [--profile <path>]
```

- `dir` defaults to `.`; profile resolution is identical to the CLI: an
  explicit `--profile` flag wins, else `ACTNG_PROFILE`, else `./actng.json`.
- **First run:** if the profile file doesn't exist, a modal offers to create
  it (name input, Enter to confirm) — the TUI equivalent of `actng init`.
  Declining exits cleanly.
- **Startup pipeline:** load profile → `import_dir` → collect + dedup into a
  `Dataset` (§5.3) → remember newly detected layouts in the profile (§5.5)
  → suggest all → land on Overview. Per-file import failures are collected,
  not fatal (visible in Files, §4.5).
- **Saving:** every profile mutation triggers `Cmd::SaveProfile` (atomic
  temp-file + rename, same helper semantics as the CLI). The status bar
  shows a save tick; there is no "unsaved changes" state to lose.

### Global keys

| Key            | Action                                          |
|----------------|--------------------------------------------------|
| `1`–`5` / `Tab`| switch screen (Overview, Review, Entries, Tags, Files) |
| `e`            | export modal (§4.6)                              |
| `R`            | re-scan `dir` from disk (new/changed files)      |
| `?`            | help overlay listing the active screen's keys    |
| `Esc`          | close modal / back                               |
| `q`            | quit (no confirm needed — everything is saved)   |

**Status bar** (bottom, always visible): profile name • tag count •
`tagged N (E exact, B bayes) · review M · dup D` • save tick • `?:help`.
The tagged/review counts are the TUI's version of the CLI `tag` summary
line and update live during review.

---

## 4. Screens

### 4.1 Overview (`1`)

The dashboard: profile card (name, version, tags, trained keys, remembered
layouts), file list with per-file entry counts and status, and the pipeline
totals. Read-only; its job is orientation and jumping off (`2` to review,
`e` to export).

### 4.2 Review (`2`) — the training loop

```
┌ Review ─ 12 of 40 ────────────────────────────────────────────┐
│  2026-03-14   -23.45   ACHAT TWINT DU 14.03 GRIMPER.CH LAUS…  │
│                                                                │
│ ┌ Candidates ─────────────┐ ┌ Queue ─────────────────────────┐│
│ │ 1  climbing       0.62  │ │▸ GRIMPER.CH LAUSANNE     -23.45││
│ │ 2  sport          0.21  │ │  MIGROS RENENS           -54.10││
│ │ 3  restaurants    0.09  │ │  SBB CFF FFS             -12.00││
│ └─────────────────────────┘ └────────────────────────────────┘│
│  1-9 confirm · t picker · n new tag · s skip · u undo · a all  │
└────────────────────────────────────────────────────────────────┘
```

- The queue is `RunResult.review` in file order: everything the tagger
  abstained on. `j`/`k`/arrows move through it; the top pane shows the
  selected entry's date, amount (negative red, positive green), and full
  description.
- **Confirm:** `1`–`9` picks a ranked candidate; `t`/`Enter` opens the tag
  picker (§4.7); `n` opens the picker pre-focused on the create row. Every
  confirmation calls `profile.learn`, saves, recomputes all suggestions,
  and advances. If the confirmation's exact key also resolves other queued
  entries (same merchant, different dates), they leave the queue in the
  same pass — toast: `climbing ✓ (+3 resolved)`.
- **Skip** (`s`) moves on without learning; skipped entries stay in the
  queue for next time.
- **Undo** (`u`): pops the session undo stack — calls `profile.unlearn`
  (§5.1), restores the exact-memory value the confirmation overwrote (the
  `UndoRecord` carries it), saves, recomputes. Multi-level within the
  session; not persisted.
- **Retag mode** (`a`): toggles the queue between review-only and *all*
  entries including confidently tagged ones — the `review --all` of the
  CLI, for correcting mistakes. Tagged entries show their current tag and
  source in the detail pane.

### 4.3 Entries (`3`)

The full deduplicated dataset as a table: date, amount, description, tag
(color-coded: exact / bayes / untagged), source file.

- `/` incremental search over descriptions; `f` cycles filter
  (all → tagged → review → per-tag).
- `Enter` retags the selected entry via the picker — same learn/save/
  recompute path as Review. This is spot-correction; bulk operations are
  out of scope for v1.

### 4.4 Tags (`4`)

```
Tag            Category     Trained   Exact keys
groceries      living           41           12
rent           living            2            1
climbing       —                 7            3
```

Backed by `Tagger::stats()` (§5.2). Keys:

- `a` add tag (name input; created with no category).
- `r` rename (input pre-filled; delegates to `rename_tag`, which rewrites
  tagger state — suggestions survive, table refreshes).
- `c` set category: picker listing existing categories plus free input, so
  category names stay consistent.
- `d` delete: confirmation modal stating exactly what is destroyed —
  *"Delete 'climbing' and its 7 trained documents? This cannot be
  undone."* — because `remove_tag` drops training data.
- `s` cycles sort (name / category / trained count).

### 4.5 Files (`5`)

The `actng scan` debugging surface, live: one row per discovered file with
entry count, date range, detected delimiter/encoding/date format, layout
provenance (`remembered` fingerprint hit vs `detected` this run), and parse
failures. Files that failed to import entirely show the error inline.
`Enter` shows the full per-file detail including the first parse failures.
Newly detected layouts are already persisted at import time (§5.5); this
screen shows that it happened.

### 4.6 Export (modal, `e` anywhere)

Form: output path (default `actng-export.csv` in `dir`), `[x] include
per-category summary` toggle. Enter writes via the shared core writer
(§5.4): columns `date, description, amount, tag, category, source_file`,
untagged entries included with an empty tag so nothing silently disappears,
proper CSV quoting. On success: toast with the path, and — if the summary
toggle was on — a modal with per-category totals (also appended to the CSV
as the CLI does).

### 4.7 Shared widgets

- **Tag picker:** modal list of all declared tags with their category
  dimmed alongside; typing filters by case-insensitive subsequence match;
  when the input is non-empty and not an existing tag name, the top row is
  `create new tag "<input>"`. Enter confirms, Esc cancels.
- **Confirm modal:** used for destructive actions (tag delete) and
  first-run profile creation.
- **Toast:** transient one-line notice (saved, resolved counts, export
  path), cleared on a timer tick or next keypress.
- **Error modal:** any `actng_core::Error` surfaced during an action
  renders as a dismissable modal; the app never panics on domain errors.

---

## 5. Core additions required (`actng-core`)

Implemented and tested in the core *before* the TUI consumes them; each is
equally useful to the CLI.

### 5.1 Unlearn (undo support)

```rust
impl NaiveBayes { pub fn untrain(&mut self, tokens: &[String], tag: &str); }
impl Tagger    { pub fn unlearn(&mut self, description: &str, tag: &str); }
impl Profile   { pub fn unlearn(&mut self, description: &str, tag: &str); }
```

`untrain` decrements the token and document counts recorded by `train`
(saturating at zero; a tag whose document count reaches zero disappears).
`Tagger::unlearn` additionally removes the exact-memory key *if it currently
maps to `tag`*. Restoring a previously overwritten exact value is the
caller's job (the TUI's `UndoRecord` carries it) — the core stays
memoryless. `Profile::unlearn` does **not** unregister the tag from `tags`;
declared tags are removed only by `remove_tag`.

### 5.2 Per-tag statistics

```rust
pub struct TagStats { pub tag: String, pub trained_docs: usize, pub exact_keys: usize }
impl Tagger { pub fn stats(&self) -> Vec<TagStats>; }  // sorted by tag
```

Powers the Tags screen and closes an existing CLI gap: `actng tags list`
is specced to show trained-doc counts (`SPEC.md` §5.2) but the core has no
accessor for them today.

### 5.3 Owned dataset

```rust
pub struct Dataset {
    pub entries: Vec<Entry>,        // deduplicated, in file order
    pub source: Vec<usize>,         // parallel: index into `sources`
    pub sources: Vec<PathBuf>,
    pub failures: Vec<(PathBuf, Error)>,
    pub duplicates_dropped: usize,
}
pub fn collect(imports: Vec<FileImport>) -> Dataset;   // module `discover`
```

`RunResult<'a>` borrows from the imports, which suits a one-shot CLI run
but not an interactive app that re-suggests after every keystroke and needs
`source_file` per entry (the export format requires it and today's
`RunResult` cannot provide it). `Profile::run` is reimplemented on top of
`collect` so the dedup logic exists once.

### 5.4 Shared export writer

```rust
// module `export`
pub fn write_csv(w: impl Write, dataset: &Dataset, profile: &Profile,
                 suggestions: &[Option<Suggestion>]) -> Result<Summary, Error>;
pub struct Summary { pub rows: usize, pub per_category: Vec<(String, f64)> }
```

Emits `date, description, amount, tag, category, source_file` with RFC-4180
quoting (via the `csv` crate already in the tree), untagged rows with empty
tag. Replaces the CLI's hand-rolled writer, which currently omits
category/source columns, drops untagged entries, and does not escape
descriptions containing commas.

### 5.5 Layout persistence helper

```rust
impl Profile {
    /// Insert fingerprint→layout for every successful import whose layout
    /// was detected rather than remembered. Returns how many were added.
    pub fn remember_layouts(&mut self, imports: &[FileImport]) -> usize;
}
```

`SPEC.md` §4.1 says "the caller may persist the result", but no caller ever
does — layouts are never written back, so fingerprint reuse never happens
in practice. The TUI calls this at startup and on re-scan (then saves); the
CLI should call it in `scan`/`tag` too.

---

## 6. Testing strategy

- **Update-loop unit tests** (the bulk): construct an `App` over fixture
  data, feed `Msg` sequences, assert on state and returned `Cmd`s — no
  terminal involved. Cover: confirm/skip/undo sequencing, auto-resolve of
  sibling keys, picker filtering and create-row behavior, delete
  confirmation flow, retag mode.
- **Render tests** with `ratatui::backend::TestBackend`: golden buffer
  assertions for each screen at 80×24 (presence of key content, not
  pixel-perfect snapshots — keep them cheap to update).
- **End-to-end session test:** temp dir with fixture CSVs + no profile →
  simulate first-run creation, a full review session, a rename, an export →
  assert the resulting profile JSON round-trips through `Profile::load` and
  the export CSV parses with the expected columns and row count.
- **Core additions** (§5) get their own unit tests in `actng-core` (untrain
  reverses train; stats counts; collect dedup parity with `Profile::run`;
  export quoting; remember_layouts idempotence).
- **Manual checklist:** resize during each screen, <80×24 fallback, Ctrl-C
  restores the terminal, profile written after kill during review.

---

## 7. Ordered implementation tasks

Same contract as `SPEC.md` §7: each task compiles, is tested, and is useful
before the next starts.

**Phase T0 — core prerequisites**

1. `Tagger::stats` (§5.2) and wire it into `actng tags list`.
2. `Dataset` + `collect`, `Profile::run` reimplemented on top (§5.3).
3. Shared `export::write_csv` (§5.4); CLI `export` switched to it (fixes
   quoting, missing columns, dropped untagged rows).
4. `Profile::remember_layouts` (§5.5); CLI `scan`/`tag` call it.
5. `untrain`/`unlearn` (§5.1).

**Phase T1 — TUI skeleton**

6. Crate scaffold: terminal lifecycle + panic hook, event loop,
   `App`/`Msg`/`update`/`view` skeleton, status bar, help overlay, quit,
   too-small fallback. Overview screen rendering real data read-only.
7. Startup pipeline (load/create-profile modal, import, collect,
   remember_layouts, suggest-all) and the Files screen.

**Phase T2 — training**

8. Tag picker widget (filtering, create row) as a reusable modal.
9. Review screen: queue, confirm via candidates and picker, new tag, skip,
   learn + atomic save + recompute + auto-resolve toast.
10. Undo stack over `unlearn`.
11. Retag mode (`a`) and the Entries browser with search/filter/retag.

**Phase T3 — management and output**

12. Tags screen: stats table, add/rename/category/delete with confirm.
13. Export modal over the shared writer, summary popup, re-scan (`R`).

**Phase T4 — hardening**

14. Update-loop test suite and TestBackend render tests (§6).
15. End-to-end session test; README section with a screenshot/cast.

---

## Appendix A — implementation review vs `SPEC.md` (2026-07-09)

Findings from auditing the current tree against the main spec; items marked
**(core)** are absorbed into §5 above, **(cli)** are CLI fixes independent
of the TUI.

- Phases A–B are complete: all fixture, accuracy, PostFinance, and module
  tests pass; `Profile`, tag management, discovery, dedup, and `run` exist
  as specced.
- `discover` deviates: recursive (spec: non-recursive v1) and accepts
  `.txt`/`.tsv` without content sniffing (spec: sniff `.txt`). Deliberate
  or not, `SPEC.md` §4.3 should be updated or the code aligned.
- **(core)** Layouts are never persisted by any caller — fingerprint reuse
  is dead code in practice (§5.5).
- **(cli)** `ACTNG_PROFILE` overrides an explicit `--profile` flag; spec
  intent is flag > env > default.
- **(cli)** Exit codes are unimplemented: `tag` never returns 2 when the
  review queue is non-empty.
- **(cli)** `tag` lacks the `N tagged (E exact, B bayes), M need review, D
  duplicates skipped` summary line.
- **(cli)** `scan` reports only entry counts — no delimiter/encoding/
  layout/date-range/parse-failure detail (the importer debugging surface).
- **(core+cli)** `export` omits `category`/`source_file` columns, silently
  drops untagged entries, and doesn't escape commas (§5.3–5.4).
- **(cli)** `tags list` lacks trained-doc counts (§5.2); `tags rm` deletes
  without the specced confirmation warning.
- **(cli)** `review` hand-rolls stdin instead of using the declared
  `dialoguer` dependency (drop the dep or use it) and has no explicit quit.
- **(cli)** No `assert_cmd`-style integration tests (spec task 17).
