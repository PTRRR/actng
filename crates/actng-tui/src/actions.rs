use std::fs;
use std::path::{Path, PathBuf};

use actng_core::{FileImport, Profile, Source};

use crate::app::{App, Picker, PickerContext, UndoRecord};

pub fn save_profile_atomically(path: &Path, profile: &Profile) -> anyhow::Result<()> {
    let temp_path = path.with_extension("tmp");
    profile.save(&temp_path)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

/// Flag > `ACTNG_PROFILE` env > `./actng.json`, identical to the CLI.
pub fn resolve_profile_path(flag: Option<&Path>) -> PathBuf {
    flag.map(Path::to_path_buf)
        .or_else(|| std::env::var_os("ACTNG_PROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("actng.json"))
}

pub fn persist_new_layouts(profile_path: &Path, profile: &mut Profile, imports: &[FileImport]) -> anyhow::Result<()> {
    if profile.remember_layouts(imports) > 0 {
        save_profile_atomically(profile_path, profile)?;
    }
    Ok(())
}

/// Confirm `tag` for the entry at `entry_idx` through the learning path. If
/// the entry carries a per-entry exception, it's cleared first via
/// `remove_override` — otherwise the stale override would silently shadow
/// this new decision (SPEC-OVERRIDES.md §3). If the entry already carried a
/// different exact-match tag (only possible in retag mode), that training
/// data is cleanly removed first via `unlearn` so no stale Bayes weight is
/// left behind — which also makes undo exact rather than approximate.
/// Returns `None` if this is a no-op re-confirmation of the same tag.
pub fn confirm_tag(app: &mut App, entry_idx: usize, tag: &str) -> Option<UndoRecord> {
    let entry = app.dataset.entries[entry_idx].clone();
    let description = entry.description.clone();
    let previous = app.profile.suggest(&description).filter(|s| s.source == Source::Exact).map(|s| s.tag);

    if previous.as_deref() == Some(tag) && app.profile.override_for(&entry).is_none() {
        return None;
    }
    app.profile.remove_override(&entry);
    if let Some(prev) = &previous {
        app.profile.unlearn(&description, prev);
    }
    app.profile.learn(&description, tag);
    app.recompute();

    Some(UndoRecord::Learn { entry_idx, description, new_tag: tag.to_string(), previous_exact_tag: previous })
}

/// Pin `tag` to the entry at `entry_idx` as a per-entry exception. Replaces
/// any previous override for the same entry without touching the tagger.
/// Returns `None` if this is a no-op re-confirmation of the same tag.
pub fn confirm_override(app: &mut App, entry_idx: usize, tag: &str) -> Option<UndoRecord> {
    let entry = app.dataset.entries[entry_idx].clone();
    let previous = app.profile.override_for(&entry).cloned();

    if previous.as_ref().map(|o| o.tag.as_str()) == Some(tag) {
        return None;
    }
    app.profile.set_override(&entry, tag);
    app.recompute();

    Some(UndoRecord::Override { entry_idx, previous })
}

/// Pop and reverse the most recent `UndoRecord`, returning the entry index
/// it applied to so the caller can move the cursor back onto it.
pub fn undo(app: &mut App) -> Option<usize> {
    let rec = app.undo.pop()?;
    let entry_idx = match rec {
        UndoRecord::Learn { entry_idx, description, new_tag, previous_exact_tag } => {
            app.profile.unlearn(&description, &new_tag);
            if let Some(prev) = previous_exact_tag {
                app.profile.learn(&description, &prev);
            }
            entry_idx
        }
        UndoRecord::Override { entry_idx, previous } => {
            let entry = app.dataset.entries[entry_idx].clone();
            app.profile.remove_override(&entry);
            if let Some(prev) = previous {
                app.profile.set_override(&entry, &prev.tag);
            }
            entry_idx
        }
    };
    app.recompute();
    Some(entry_idx)
}

/// One row of a `Picker`'s filtered list.
#[derive(Debug, Clone)]
pub enum PickerItem {
    /// An existing declared tag, with its category shown dimmed alongside.
    Tag { name: String, category: Option<String> },
    /// An existing category name (used by the `Category` picker context).
    Category(String),
    /// `query` isn't an existing item: offer to create it, shown first.
    Create(String),
}

impl PickerItem {
    pub fn label(&self) -> &str {
        match self {
            PickerItem::Tag { name, .. } => name,
            PickerItem::Category(c) => c,
            PickerItem::Create(q) => q,
        }
    }
}

/// The filtered, ranked list of rows a picker currently shows.
pub fn picker_items(app: &App, picker: &Picker) -> Vec<PickerItem> {
    let query = picker.query.trim();
    let mut rows: Vec<PickerItem> = match &picker.context {
        PickerContext::Tag { .. } | PickerContext::Override { .. } => {
            let mut scored: Vec<(i32, PickerItem)> = app
                .profile
                .tags
                .iter()
                .filter_map(|t| {
                    fuzzy_score(query, &t.name)
                        .map(|score| (score, PickerItem::Tag { name: t.name.clone(), category: t.category.clone() }))
                })
                .collect();
            scored.sort_by_key(|(score, item)| (*score, item.label().to_string()));
            scored.into_iter().map(|(_, item)| item).collect()
        }
        PickerContext::Category { .. } => {
            let mut categories: Vec<String> =
                app.profile.tags.iter().filter_map(|t| t.category.clone()).collect();
            categories.sort();
            categories.dedup();
            let mut scored: Vec<(i32, PickerItem)> = categories
                .into_iter()
                .filter_map(|c| fuzzy_score(query, &c).map(|score| (score, PickerItem::Category(c))))
                .collect();
            scored.sort_by_key(|(score, item)| (*score, item.label().to_string()));
            scored.into_iter().map(|(_, item)| item).collect()
        }
    };

    let exact_exists = rows.iter().any(|r| r.label().eq_ignore_ascii_case(query));
    if !query.is_empty() && !exact_exists {
        rows.insert(0, PickerItem::Create(query.to_string()));
    }
    rows
}

/// Case-insensitive subsequence match, scored by how tightly the matched
/// characters cluster (lower is better) so closer matches sort first. Not a
/// crate-grade fuzzy matcher — just enough to filter a few dozen tag names.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let query: Vec<char> = query.to_lowercase().chars().collect();
    let candidate: Vec<char> = candidate.to_lowercase().chars().collect();

    let mut qi = 0;
    let mut first_match = None;
    let mut last_match = 0;
    for (ci, &c) in candidate.iter().enumerate() {
        if qi < query.len() && c == query[qi] {
            if first_match.is_none() {
                first_match = Some(ci);
            }
            last_match = ci;
            qi += 1;
        }
    }
    if qi < query.len() {
        return None;
    }
    let span = last_match - first_match.unwrap_or(0);
    Some(span as i32)
}
