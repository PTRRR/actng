use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::import::ImportProfile;
use crate::tagger::{Suggestion, Tagger};

/// Current on-disk format version. Bump when the schema changes and add a
/// migration path in [`Profile::load`]; older files stay loadable this way,
/// newer ones are rejected outright since this build can't understand them.
pub const CURRENT_VERSION: u32 = 1;

/// A user-declared tag: a name, its category (for grouping/summary export),
/// and an optional free-text note. Categories aren't a separate concept —
/// they're just a field on each tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub category: Option<String>,
    pub description: Option<String>,
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

    pub fn suggest(&self, description: &str) -> Option<Suggestion> {
        self.tagger.suggest(description)
    }

    pub fn candidates(&self, description: &str) -> Vec<(String, f64)> {
        self.tagger.candidates(description)
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
        Ok(())
    }

    /// Remove a declared tag and drop all training data recorded under it.
    pub fn remove_tag(&mut self, tag: &str) {
        self.tags.retain(|t| t.name != tag);
        self.tagger.remove_tag(tag);
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
}
