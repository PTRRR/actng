# actng — per-entry tag overrides (exceptions)

Companion to `SPEC.md` and `SPEC-TUI.md`. Adds a way to give one specific
entry a tag **without training** the tagger: an *override*, or exception.

Everything here is **[done]**, with one deliberate deviation: see the
schema-versioning note in §2.

---

## 1. Motivation

The pipeline (`SPEC.md` §3) deliberately keys learned tags on the
*normalized* description — dates, amounts, card masks and reference numbers
are stripped so the same merchant matches across statements. That is the
right default, and it has a known blind spot, already called out in
`SPEC.md` §3:

> Two entries with the same normalized key can only carry one tag — last
> confirmation wins.

Real case: two purchases at the same shop, one is `groceries`, the other a
`gift`. Confirming `gift` through the review loop would (a) flip the
exact-memory entry so *every* purchase at that shop now suggests `gift`,
and (b) pollute the Bayes counts with a one-off. What the user means is
"this one entry is different" — an exception, not a lesson.

### Goals

- **Pin a tag to one concrete entry.** The assignment survives re-imports
  and re-runs, like everything else in the profile.
- **Zero training side effects.** Neither the exact memory nor the Bayes
  counts change. The tagger's behavior for every other entry is untouched.
- **Profile-portable.** Overrides live in the profile JSON next to the
  tagger state, human-readable and hand-editable.
- **Core purity preserved.** Resolution order and override storage are
  core concerns; frontends only choose *when* to create one.

### Non-goals

- No amount- or date-*range* rules ("everything above 100 CHF is X") — an
  override names one concrete entry, it is not a rule language.
- No override-only workflow. Exceptions are the escape hatch; training
  stays the primary path and the review queue is unaffected.
- No per-entry notes/annotations — tag only.

---

## 2. Data model

An entry has no stable ID: the dataset is regenerated from the CSVs on
every run and row order across bank re-exports is not trustworthy. The
identity that *does* survive is the raw imported triple:

```rust
/// A pinned tag for one concrete entry, matched on the raw imported
/// values (not the normalized key — normalization is exactly what makes
/// same-merchant entries collide).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Override {
    pub date: Option<NaiveDate>,
    pub description: String,     // raw, as imported
    pub amount: Option<f64>,
    pub tag: String,
}
```

`Profile` gains:

```rust
pub struct Profile {
    // ... existing fields ...
    /// Per-entry exceptions, checked before the tagger. Matched on the
    /// exact (date, description, amount) triple.
    #[serde(default)]
    pub overrides: Vec<Override>,
}
```

- Stored as plain structs, not hashes: the profile stays greppable and a
  stale override (statement file deleted) is diagnosable by eye.
- At most one override per triple — setting a new one for the same triple
  replaces the old.

### Schema versioning

**Deviation from the original plan:** `CURRENT_VERSION` was *not* bumped.
The original design called for **1 → 2**, specifically to guard against a
version-1 *build* opening a version-2 profile and silently dropping every
override on its next save (serde ignores unknown fields, so without a
version bump that data loss goes undetected). That protects a multi-binary
deployment where an old build might still touch a newer profile file.
For a single-user tool where only one build is ever in play, that failure
mode doesn't arise, so the bump was skipped to keep `actng.json` files
portable across this and the previous version without a migration step.
`overrides` still defaults to empty via `#[serde(default)]`, so old
profiles load unchanged either way. Revisit the bump if this profile is
ever opened by more than one build at a time.

### Identity caveat

Two *truly* identical entries — same date, same raw description, same
amount (two identical coffees in one day) — cannot be told apart by any
key that survives a re-export. v1 semantics: **an override applies to every
entry matching the triple.** Documented; an occurrence index can be added
later if it ever matters in practice.

---

## 3. Resolution order and core API

Suggestion lookup becomes, per entry:

```
Override  →  Exact memory  →  Naive Bayes  →  review queue
```

`Source` gains a variant so frontends can always show *why* an entry got
its tag:

```rust
pub enum Source { Override, Exact, Bayes }
```

An override suggestion carries `confidence: 1.0`, like an exact hit.

### `Profile` methods

Override resolution needs the full entry, not just the description, so it
lives on `Profile`, not `Tagger` — the tagger stays a pure
description→tag engine and is untouched by this feature:

```rust
impl Profile {
    /// Override-aware suggestion: checks `overrides` first, then
    /// delegates to the tagger. `Profile::run` switches to this.
    pub fn suggest_entry(&self, entry: &Entry) -> Option<Suggestion>;

    /// Pin `tag` to this entry's (date, description, amount) triple,
    /// replacing any previous override for the same triple.
    /// Auto-registers `tag` in `tags` (like `learn`) so category
    /// assignment and export work uniformly. No tagger mutation.
    pub fn set_override(&mut self, entry: &Entry, tag: &str);

    /// Remove the override matching this entry's triple, if any.
    /// Returns the removed override (frontends need it for undo).
    pub fn remove_override(&mut self, entry: &Entry) -> Option<Override>;

    /// The override matching this entry's triple, if any.
    pub fn override_for(&self, entry: &Entry) -> Option<&Override>;
}
```

`f64` amounts compare with `==` here — both sides came through the same
parser from the same file text, so they are bit-identical; no epsilon.

### Interaction rules

- **`Profile::run`** uses `suggest_entry`; overridden entries land in
  `tagged` with `Source::Override` and never enter the review queue.
- **Override beats a later `learn`.** Training continues to work normally
  on the merchant; the pinned entry keeps its exception.
- **Normal retag clears the exception.** When a frontend retags an entry
  that carries an override *through the learning path*, it must call
  `remove_override(entry)` before `learn` — otherwise the stale override
  would shadow the new decision invisibly. (Core keeps the two calls
  explicit; frontends own the sequencing, as with learn/save today.)
- **`rename_tag`** rewrites matching `Override.tag` values, same as it
  rewrites exact-memory values.
- **`remove_tag`** drops every override carrying that tag, same as it
  drops the tag's training data. The delete-confirmation copy states it
  (§5, Tags screen).
- **Export** needs no changes: overrides flow through `Suggestion` into
  `write_csv`, tag and category included, summary buckets as usual.

---

## 4. CLI surface

- **`actng review`** — the dialoguer select gains one item alongside
  "new tag…"/"skip": **"tag as exception…"**, which opens the same tag
  prompt but calls `set_override` instead of `learn` (then saves). The
  queue advances as usual; other queued entries with the same normalized
  key are *not* resolved — that's the point.
- **`actng tag` / summary lines** — the tagged breakdown gains the new
  source: `312 tagged (287 exact, 25 bayes, 3 override), 40 need review`.
- **`actng overrides list`** — one row per override: date, amount,
  description (truncated), tag. The audit surface for "why is this entry
  tagged X".
- **`actng overrides rm <index>`** — remove by the index `list` printed.
  Indices are only stable within one `list` output; acceptable for a
  hand-maintenance command.
- **`actng profile-info`** — adds the override count.

---

## 5. TUI surface

(`e` is taken globally for export; the exception key is `x`.)

- **Review screen (`2`)** — new key `x`: opens the tag picker (§4.7 of
  `SPEC-TUI.md`), but confirms via `set_override` instead of
  `profile.learn`. Toast: `gift ✓ (exception)` — no `+N resolved`, since
  exceptions never cascade. Footer becomes
  `1-9 confirm · t picker · n new · x exception · s skip · u undo · a all`.
- **Entries screen (`3`)** — the *main* entry point: an entry whose
  learned suggestion is right in general but wrong for this occurrence is
  confidently tagged and never reaches the review queue, so Entries is
  where the user finds it. `Enter` keeps its learn semantics (and now
  clears a stale override first, per §3); new key `x` retags the selected
  entry as an exception. Overridden entries get their own color in the tag
  column (override / exact / bayes / untagged) and `f` gains an
  `overridden` filter stop.
- **Undo (`u`)** — the session `UndoRecord` gains an override variant:
  undoing an exception calls `remove_override` and restores the previous
  override if `set_override` replaced one. Same stack, same multi-level
  semantics.
- **Tags screen (`4`)** — delete confirmation copy extends to
  *"Delete 'climbing', its 7 trained documents and 2 exceptions? This
  cannot be undone."* An `Overrides` count column is added next to
  `Exact keys`.
- **Status bar** — tagged breakdown includes overrides when non-zero:
  `tagged N (E exact, B bayes, O ovr)`.

---

## 6. Testing

Core (`profile.rs` / new tests):

- `set_override` → `suggest_entry` returns the pinned tag with
  `Source::Override`; the tagger's own `suggest` for the same description
  is unchanged (no training side effect — the core promise).
- Two entries, same normalized key, different (date, amount): one
  overridden, one not — `run` tags them differently; the non-overridden
  one still follows the exact memory.
- `set_override` on the same triple twice replaces, not duplicates.
- `remove_override` returns the removed value; `suggest_entry` falls back
  to the tagger afterwards.
- `rename_tag` rewrites override tags; `remove_tag` drops them.
- Round-trip: profile with overrides saves and loads (version 2); a
  version-1 file loads with empty overrides; a version-3 file is rejected.
- `write_csv`: overridden entry exports with its pinned tag and that tag's
  category; summary buckets accordingly.

CLI: integration test — scripted review choosing "tag as exception",
re-run `tag`, assert the entry is tagged with source `override` and the
sibling entry (same merchant) is unaffected; `overrides list`/`rm`
round-trip.

TUI: update-loop tests (as in `update.rs` today) — `x` in Review and
Entries creates an override and advances; `u` undoes it; retag via `Enter`
on an overridden entry clears the override.

---

## 7. Ordered implementation tasks

1. **Core: `Override` type + `Profile.overrides`**, version bump 1 → 2
   with default migration, round-trip and version tests.
2. **Core: resolution** — `Source::Override`, `suggest_entry`,
   `set_override` / `remove_override` / `override_for`, `Profile::run`
   switched to `suggest_entry`. Interaction tests (§6).
3. **Core: tag-op integration** — `rename_tag` / `remove_tag` cover
   overrides; export test.
4. **CLI**: review "tag as exception…", `overrides list|rm`, summary-line
   and `profile-info` counts, integration test.
5. **TUI**: `x` in Review and Entries, undo variant, Enter-retag clears
   override, tag-column color + filter stop, Tags-screen count and delete
   copy, status-bar count, update-loop tests.
6. **Docs**: README hotkeys and CLI reference; fold the §3 caveat note in
   `SPEC.md` into a pointer to this file.
