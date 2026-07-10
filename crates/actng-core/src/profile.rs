use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::discover::{Dataset, FileImport};
use crate::entry::Entry;
use crate::error::Error;
use crate::import::ImportProfile;
use crate::tagger::{Source, Suggestion, Tagger};

/// Current on-disk format version. Bump when the schema changes and add a
/// migration path in [`Profile::load`]; older files stay loadable this way,
/// newer ones are rejected outright since this build can't understand them.
pub const CURRENT_VERSION: u32 = 1;

/// A pinned tag for one concrete entry, matched on the raw imported values
/// (not the normalized key — normalization is exactly what makes
/// same-merchant entries collide).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Override {
    pub date: Option<NaiveDate>,
    pub description: String,
    pub amount: Option<f64>,
    pub tag: String,
}

impl Override {
    fn matches(&self, entry: &Entry) -> bool {
        self.date == entry.date && self.description == entry.description && self.amount == entry.amount
    }
}

/// A user-declared tag: a name, its category (for grouping/summary export),
/// and an optional free-text note. Categories aren't a separate concept —
/// they're just a field on each tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub category: Option<String>,
    pub description: Option<String>,
}

/// A transaction that the tagger could not confidently label.
#[derive(Debug)]
pub struct Review<'a> {
    pub entry: &'a crate::entry::Entry,
    pub candidates: Vec<(String, f64)>,
}

/// The total result of applying a `Profile` to a set of imports.
#[derive(Debug)]
pub struct RunResult<'a> {
    pub tagged: Vec<(&'a crate::entry::Entry, Suggestion)>,
    pub review: Vec<Review<'a>>,
    pub sources: Vec<PathBuf>,
}

/// Everything the user has taught the tool, in one serializable artifact:
/// the declared tag set, the trained tagger (exact memory + Bayes weights),
/// and remembered CSV layouts. Copy the file to another machine and keep
/// training there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub version: u32,
    pub name: String,
    pub tags: Vec<Tag>,
    pub tagger: Tagger,
    /// Remembered CSV layouts keyed by file fingerprint (see the `discover`
    /// module), so a re-export from the same bank never re-runs (or
    /// mis-runs) auto-detection.
    pub layouts: HashMap<String, ImportProfile>,
    /// Per-entry exceptions, checked before the tagger. Matched on the exact
    /// (date, description, amount) triple.
    #[serde(default)]
    pub overrides: Vec<Override>,
}

impl Profile {
    /// An empty profile with a default tagger.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: CURRENT_VERSION,
            name: name.into(),
            tags: Vec::new(),
            tagger: Tagger::default(),
            layouts: HashMap::new(),
            overrides: Vec::new(),
        }
    }

    /// Load a profile from pretty-printed JSON. Rejects a file whose
    /// `version` is newer than [`CURRENT_VERSION`] with a clear error;
    /// older versions would be migrated here once the format ever changes.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path)?;
        let profile: Profile = serde_json::from_str(&text)?;
        if profile.version > CURRENT_VERSION {
            return Err(Error::UnsupportedVersion { found: profile.version, current: CURRENT_VERSION });
        }
        Ok(profile)
    }

    /// Save as pretty-printed JSON.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), Error> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Record a confirmed tag for a description, and auto-register the tag
    /// (with no category yet) if it's new.
    pub fn learn(&mut self, description: &str, tag: &str) {
        self.tagger.learn(description, tag);
        self.add_tag(tag);
    }

    /// Reverse one `learn` call (see [`Tagger::unlearn`]). Does not
    /// unregister `tag` from `self.tags` — only `remove_tag` does that.
    pub fn unlearn(&mut self, description: &str, tag: &str) {
        self.tagger.unlearn(description, tag);
    }

    pub fn suggest(&self, description: &str) -> Option<Suggestion> {
        self.tagger.suggest(description)
    }

    pub fn candidates(&self, description: &str) -> Vec<(String, f64)> {
        self.tagger.candidates(description)
    }

    /// Override-aware suggestion: checks `overrides` first, then delegates
    /// to the tagger.
    pub fn suggest_entry(&self, entry: &Entry) -> Option<Suggestion> {
        if let Some(ovr) = self.override_for(entry) {
            return Some(Suggestion { tag: ovr.tag.clone(), confidence: 1.0, source: Source::Override });
        }
        self.suggest(&entry.description)
    }

    /// Pin `tag` to this entry's (date, description, amount) triple,
    /// replacing any previous override for the same triple. Auto-registers
    /// `tag` in `tags` (like `learn`) so category assignment and export work
    /// uniformly. No tagger mutation.
    pub fn set_override(&mut self, entry: &Entry, tag: &str) {
        self.overrides.retain(|o| !o.matches(entry));
        self.overrides.push(Override {
            date: entry.date,
            description: entry.description.clone(),
            amount: entry.amount,
            tag: tag.to_string(),
        });
        self.add_tag(tag);
    }

    /// Remove the override matching this entry's triple, if any. Returns the
    /// removed override (frontends need it for undo).
    pub fn remove_override(&mut self, entry: &Entry) -> Option<Override> {
        let pos = self.overrides.iter().position(|o| o.matches(entry))?;
        Some(self.overrides.remove(pos))
    }

    /// The override matching this entry's triple, if any.
    pub fn override_for(&self, entry: &Entry) -> Option<&Override> {
        self.overrides.iter().find(|o| o.matches(entry))
    }

    /// Declare a tag with no category, if it isn't already known.
    pub fn add_tag(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !self.tags.iter().any(|t| t.name == name) {
            self.tags.push(Tag { name, category: None, description: None });
        }
    }

    pub fn set_category(&mut self, tag: &str, category: impl Into<String>) -> Result<(), Error> {
        let t = self.tags.iter_mut().find(|t| t.name == tag).ok_or_else(|| Error::UnknownTag(tag.to_string()))?;
        t.category = Some(category.into());
        Ok(())
    }

    /// Rename a tag, rewriting every exact-match entry and Bayes count the
    /// tagger recorded under the old name. If `new` is already a distinct
    /// declared tag, the two merge (their training data was already merged
    /// by the tagger; the duplicate `Tag` entry is dropped).
    pub fn rename_tag(&mut self, old: &str, new: &str) -> Result<(), Error> {
        if old == new {
            return Ok(());
        }
        let pos =
            self.tags.iter().position(|t| t.name == old).ok_or_else(|| Error::UnknownTag(old.to_string()))?;
        self.tags[pos].name = new.to_string();
        if let Some(dup) = self.tags.iter().enumerate().position(|(i, t)| i != pos && t.name == new) {
            self.tags.remove(dup);
        }
        self.tagger.rename_tag(old, new);
        for ovr in &mut self.overrides {
            if ovr.tag == old {
                ovr.tag = new.to_string();
            }
        }
        Ok(())
    }

    /// Remove a declared tag, drop all training data recorded under it, and
    /// drop every override pinning it.
    pub fn remove_tag(&mut self, tag: &str) {
        self.tags.retain(|t| t.name != tag);
        self.tagger.remove_tag(tag);
        self.overrides.retain(|o| o.tag != tag);
    }

    /// Insert `fingerprint -> layout` for every successful import whose
    /// layout isn't already remembered, so a re-export from the same bank
    /// never re-runs (or mis-runs) auto-detection. Returns how many layouts
    /// were newly remembered. Never overwrites an existing entry.
    pub fn remember_layouts(&mut self, imports: &[FileImport]) -> usize {
        let mut added = 0;
        for imp in imports {
            if let Ok(import) = &imp.result {
                if !import.fingerprint.is_empty() && !self.layouts.contains_key(&import.fingerprint) {
                    self.layouts.insert(import.fingerprint.clone(), import.profile.clone());
                    added += 1;
                }
            }
        }
        added
    }

    /// Apply the profile's tagger to a deduplicated `Dataset`, partitioning
    /// results into confident matches and those needing human review.
    /// Dedup itself happens once, in `discover::collect`.
    pub fn run<'a>(&self, dataset: &'a Dataset) -> RunResult<'a> {
        let mut tagged = Vec::new();
        let mut review = Vec::new();

        for entry in &dataset.entries {
            if let Some(sugg) = self.suggest_entry(entry) {
                tagged.push((entry, sugg));
            } else {
                review.push(Review { entry, candidates: self.candidates(&entry.description) });
            }
        }

        RunResult { tagged, review, sources: dataset.sources.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learn_auto_registers_new_tags() {
        let mut profile = Profile::new("personal");
        profile.learn("COOP LAUSANNE", "groceries");
        assert_eq!(profile.tags, vec![Tag { name: "groceries".into(), category: None, description: None }]);
        assert_eq!(profile.suggest("COOP LAUSANNE").unwrap().tag, "groceries");

        // Learning the same tag again doesn't duplicate it.
        profile.learn("MIGROS RENENS", "groceries");
        assert_eq!(profile.tags.len(), 1);
    }

    #[test]
    fn round_trips_through_json() {
        let dir = std::env::temp_dir().join(format!("actng-profile-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("profile.json");

        let mut profile = Profile::new("personal");
        profile.learn("COOP LAUSANNE", "groceries");
        profile.set_category("groceries", "living").unwrap();
        profile.save(&path).unwrap();

        let restored = Profile::load(&path).unwrap();
        assert_eq!(restored.name, "personal");
        assert_eq!(restored.tags[0].category.as_deref(), Some("living"));
        assert_eq!(restored.suggest("COOP LAUSANNE").unwrap().tag, "groceries");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_newer_version() {
        let dir = std::env::temp_dir().join(format!("actng-profile-version-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("profile.json");

        let profile = Profile::new("x");
        let mut value = serde_json::to_value(&profile).unwrap();
        value["version"] = serde_json::json!(CURRENT_VERSION + 1);
        std::fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();

        let err = Profile::load(&path).unwrap_err();
        assert!(matches!(err, Error::UnsupportedVersion { found, current } if found == CURRENT_VERSION + 1 && current == CURRENT_VERSION));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_tag_preserves_suggestions_and_declared_tag() {
        let mut profile = Profile::new("personal");
        profile.learn("COOP LAUSANNE", "groceries");
        profile.rename_tag("groceries", "food").unwrap();

        assert_eq!(profile.tags, vec![Tag { name: "food".into(), category: None, description: None }]);
        assert_eq!(profile.suggest("COOP LAUSANNE").unwrap().tag, "food");
    }

    #[test]
    fn remove_tag_drops_training_data() {
        let mut profile = Profile::new("personal");
        profile.learn("COOP LAUSANNE", "groceries");
        profile.remove_tag("groceries");

        assert!(profile.tags.is_empty());
        assert!(profile.suggest("COOP LAUSANNE").is_none());
    }

    fn entry(date: Option<NaiveDate>, description: &str, amount: Option<f64>) -> Entry {
        Entry { date, description: description.to_string(), amount, raw: vec![] }
    }

    #[test]
    fn set_override_pins_tag_without_training_side_effects() {
        let mut profile = Profile::new("personal");
        let e = entry(NaiveDate::from_ymd_opt(2025, 1, 1), "COOP LAUSANNE", Some(-10.0));

        profile.set_override(&e, "gift");

        let sugg = profile.suggest_entry(&e).unwrap();
        assert_eq!(sugg.tag, "gift");
        assert_eq!(sugg.source, Source::Override);
        assert_eq!(sugg.confidence, 1.0);

        // The tagger itself was never trained: a plain description lookup
        // (the description-only path other entries at the same merchant
        // would hit) sees nothing.
        assert!(profile.suggest("COOP LAUSANNE").is_none());
        assert!(profile.tags.iter().any(|t| t.name == "gift"));
    }

    #[test]
    fn override_applies_to_one_entry_not_its_normalized_siblings() {
        let mut profile = Profile::new("personal");
        profile.learn("COOP LAUSANNE", "groceries");

        let overridden = entry(NaiveDate::from_ymd_opt(2025, 1, 1), "COOP LAUSANNE", Some(-10.0));
        let sibling = entry(NaiveDate::from_ymd_opt(2025, 1, 2), "COOP LAUSANNE", Some(-20.0));

        profile.set_override(&overridden, "gift");

        assert_eq!(profile.suggest_entry(&overridden).unwrap().tag, "gift");
        assert_eq!(profile.suggest_entry(&overridden).unwrap().source, Source::Override);

        let sibling_sugg = profile.suggest_entry(&sibling).unwrap();
        assert_eq!(sibling_sugg.tag, "groceries");
        assert_eq!(sibling_sugg.source, Source::Exact);
    }

    #[test]
    fn set_override_on_same_triple_twice_replaces_not_duplicates() {
        let mut profile = Profile::new("personal");
        let e = entry(NaiveDate::from_ymd_opt(2025, 1, 1), "COOP LAUSANNE", Some(-10.0));

        profile.set_override(&e, "gift");
        profile.set_override(&e, "climbing");

        assert_eq!(profile.overrides.len(), 1);
        assert_eq!(profile.suggest_entry(&e).unwrap().tag, "climbing");
    }

    #[test]
    fn remove_override_returns_removed_value_and_falls_back_to_tagger() {
        let mut profile = Profile::new("personal");
        profile.learn("COOP LAUSANNE", "groceries");
        let e = entry(NaiveDate::from_ymd_opt(2025, 1, 1), "COOP LAUSANNE", Some(-10.0));
        profile.set_override(&e, "gift");

        let removed = profile.remove_override(&e).unwrap();
        assert_eq!(removed.tag, "gift");
        assert!(profile.remove_override(&e).is_none());

        let sugg = profile.suggest_entry(&e).unwrap();
        assert_eq!(sugg.tag, "groceries");
        assert_eq!(sugg.source, Source::Exact);
    }

    #[test]
    fn rename_tag_rewrites_override_tags() {
        let mut profile = Profile::new("personal");
        let e = entry(NaiveDate::from_ymd_opt(2025, 1, 1), "COOP LAUSANNE", Some(-10.0));
        profile.set_override(&e, "groceries");

        profile.rename_tag("groceries", "food").unwrap();

        assert_eq!(profile.overrides[0].tag, "food");
        assert_eq!(profile.suggest_entry(&e).unwrap().tag, "food");
    }

    #[test]
    fn remove_tag_drops_overrides_pinning_it() {
        let mut profile = Profile::new("personal");
        let e = entry(NaiveDate::from_ymd_opt(2025, 1, 1), "COOP LAUSANNE", Some(-10.0));
        profile.set_override(&e, "gift");

        profile.remove_tag("gift");

        assert!(profile.overrides.is_empty());
        assert!(profile.suggest_entry(&e).is_none());
    }

    #[test]
    fn overrides_round_trip_through_json() {
        let dir = std::env::temp_dir().join(format!("actng-override-roundtrip-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("profile.json");

        let mut profile = Profile::new("personal");
        let e = entry(NaiveDate::from_ymd_opt(2025, 1, 1), "COOP LAUSANNE", Some(-10.0));
        profile.set_override(&e, "gift");
        profile.save(&path).unwrap();

        let restored = Profile::load(&path).unwrap();
        assert_eq!(restored.overrides.len(), 1);
        assert_eq!(restored.suggest_entry(&e).unwrap().tag, "gift");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn profile_without_overrides_field_loads_with_empty_vec() {
        let dir = std::env::temp_dir().join(format!("actng-override-migration-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("profile.json");

        let profile = Profile::new("personal");
        let mut value = serde_json::to_value(&profile).unwrap();
        value.as_object_mut().unwrap().remove("overrides");
        std::fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();

        let restored = Profile::load(&path).unwrap();
        assert!(restored.overrides.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remember_layouts_persists_detected_layouts_and_is_idempotent() {
        use crate::discover::import_dir;

        let dir = std::env::temp_dir().join(format!("actng-remember-layouts-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.csv"), "Date,Desc,Amount\n2025-01-01,Test,-10.00").unwrap();

        let mut profile = Profile::new("test");
        assert!(profile.layouts.is_empty());

        let imports = import_dir(&dir, &profile).unwrap();
        let added = profile.remember_layouts(&imports);
        assert_eq!(added, 1);
        assert_eq!(profile.layouts.len(), 1);

        // A second run detects the same fingerprint; nothing new to add.
        let imports2 = import_dir(&dir, &profile).unwrap();
        let added2 = profile.remember_layouts(&imports2);
        assert_eq!(added2, 0);
        assert_eq!(profile.layouts.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }
}
