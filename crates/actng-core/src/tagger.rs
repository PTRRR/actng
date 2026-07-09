use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::bayes::NaiveBayes;
use crate::error::Error;
use crate::normalize::normalize;

/// Where a suggestion came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// The normalized token set was tagged before — deterministic recall.
    Exact,
    /// Probabilistic suggestion from the Naive Bayes classifier.
    Bayes,
}

/// A tag proposed for an entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub tag: String,
    /// Posterior probability; `1.0` for exact matches.
    pub confidence: f64,
    pub source: Source,
}

/// Training volume for one tag: how many documents fed the Bayes model and
/// how many normalized keys resolve to it by exact match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagStats {
    pub tag: String,
    pub trained_docs: u64,
    pub exact_keys: usize,
}

/// The tagging engine: exact-match memory first, Naive Bayes fallback.
///
/// [`learn`](Tagger::learn) records a user decision in both layers, so the
/// same entry never asks twice (exact) and similar entries get increasingly
/// good suggestions (Bayes). Serializable with serde for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tagger {
    exact: HashMap<String, String>,
    bayes: NaiveBayes,
    /// Bayes suggestions below this posterior are suppressed (entry should go
    /// to a review queue instead).
    pub min_confidence: f64,
}

impl Default for Tagger {
    fn default() -> Self {
        Self { exact: HashMap::new(), bayes: NaiveBayes::new(), min_confidence: 0.8 }
    }
}

impl Tagger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_min_confidence(min_confidence: f64) -> Self {
        Self { min_confidence, ..Self::default() }
    }

    /// Suggest a tag for a raw description, or `None` if the entry needs
    /// human review (no evidence, or confidence below the threshold).
    pub fn suggest(&self, description: &str) -> Option<Suggestion> {
        let n = normalize(description);
        if let Some(tag) = self.exact.get(&n.key) {
            return Some(Suggestion { tag: tag.clone(), confidence: 1.0, source: Source::Exact });
        }
        let scores = self.bayes.classify(&n.tokens);
        let (tag, confidence) = scores.into_iter().next()?;
        (confidence >= self.min_confidence)
            .then_some(Suggestion { tag, confidence, source: Source::Bayes })
    }

    /// All candidate tags ranked by posterior, ignoring the confidence
    /// threshold — useful to pre-fill choices in a review UI.
    pub fn candidates(&self, description: &str) -> Vec<(String, f64)> {
        let n = normalize(description);
        if let Some(tag) = self.exact.get(&n.key) {
            return vec![(tag.clone(), 1.0)];
        }
        self.bayes.classify(&n.tokens)
    }

    /// Record a confirmed tag for a description. Overwrites any previous
    /// exact-match entry for the same normalized key and trains the classifier.
    pub fn learn(&mut self, description: &str, tag: &str) {
        let n = normalize(description);
        if !n.key.is_empty() {
            self.exact.insert(n.key, tag.to_string());
        }
        self.bayes.train(&n.tokens, tag);
    }

    /// Reverse one `learn` call: drop the exact-match entry for `description`
    /// if it still maps to `tag` (a later `learn` for the same description
    /// under a different tag is left alone), and untrain the classifier.
    /// Does not unregister `tag` from any declared tag list — that's
    /// `Profile::remove_tag`'s job.
    pub fn unlearn(&mut self, description: &str, tag: &str) {
        let n = normalize(description);
        if self.exact.get(&n.key).map(String::as_str) == Some(tag) {
            self.exact.remove(&n.key);
        }
        self.bayes.untrain(&n.tokens, tag);
    }

    /// Rewrite every exact-match entry and Bayes count recorded under `old`
    /// to `new` (used when a tag is renamed).
    pub fn rename_tag(&mut self, old: &str, new: &str) {
        for value in self.exact.values_mut() {
            if value == old {
                *value = new.to_string();
            }
        }
        self.bayes.rename_tag(old, new);
    }

    /// Drop every exact-match entry and Bayes count recorded under `tag`
    /// (used when a tag is deleted; its training data goes with it).
    pub fn remove_tag(&mut self, tag: &str) {
        self.exact.retain(|_, v| v != tag);
        self.bayes.remove_tag(tag);
    }

    /// Distinct tags known to the engine, sorted.
    pub fn tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self.bayes.tags().map(str::to_string).collect();
        for tag in self.exact.values() {
            if !tags.iter().any(|t| t == tag) {
                tags.push(tag.clone());
            }
        }
        tags.sort();
        tags
    }

    /// Per-tag training volume, sorted by tag name.
    pub fn stats(&self) -> Vec<TagStats> {
        let mut exact_keys: HashMap<&str, usize> = HashMap::new();
        for tag in self.exact.values() {
            *exact_keys.entry(tag.as_str()).or_default() += 1;
        }
        let mut tags = self.tags();
        tags.sort();
        tags.into_iter()
            .map(|tag| {
                let trained_docs = self.bayes.doc_count(&tag);
                let exact_keys = exact_keys.get(tag.as_str()).copied().unwrap_or(0);
                TagStats { tag, trained_docs, exact_keys }
            })
            .collect()
    }

    pub fn to_json(&self) -> Result<String, Error> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(json: &str) -> Result<Self, Error> {
        Ok(serde_json::from_str(json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_beats_bayes_and_survives_noise() {
        let mut tagger = Tagger::new();
        tagger.learn("ACHAT/PRESTATION TWINT DU 06.11.2025 GRIMPER.CH LAUSANNE (CH)", "climbing");

        // Same merchant on another date: only noise differs, exact hit.
        let s = tagger
            .suggest("ACHAT/PRESTATION TWINT DU 21.10.2025 GRIMPER.CH LAUSANNE (CH)")
            .unwrap();
        assert_eq!(s.tag, "climbing");
        assert_eq!(s.source, Source::Exact);
        assert_eq!(s.confidence, 1.0);
    }

    #[test]
    fn unknown_entries_get_no_suggestion() {
        let mut tagger = Tagger::new();
        tagger.learn("COOP LAUSANNE", "groceries");
        assert!(tagger.suggest("SOMETHING ENTIRELY DIFFERENT").is_none());
    }

    #[test]
    fn unlearn_reverses_learn() {
        let mut tagger = Tagger::new();
        tagger.learn("GRIMPER.CH LAUSANNE", "climbing");
        assert!(tagger.suggest("GRIMPER.CH LAUSANNE").is_some());

        tagger.unlearn("GRIMPER.CH LAUSANNE", "climbing");
        assert!(tagger.suggest("GRIMPER.CH LAUSANNE").is_none());
        assert!(!tagger.tags().contains(&"climbing".to_string()));
    }

    #[test]
    fn unlearn_leaves_a_later_overwrite_alone() {
        let mut tagger = Tagger::new();
        tagger.learn("GRIMPER.CH LAUSANNE", "climbing");
        tagger.learn("GRIMPER.CH LAUSANNE", "sport"); // user corrected their mind

        // Undoing the stale "climbing" confirmation must not clobber "sport".
        tagger.unlearn("GRIMPER.CH LAUSANNE", "climbing");
        assert_eq!(tagger.suggest("GRIMPER.CH LAUSANNE").unwrap().tag, "sport");
    }

    #[test]
    fn stats_reports_trained_docs_and_exact_keys() {
        let mut tagger = Tagger::new();
        tagger.learn("COOP LAUSANNE", "groceries");
        tagger.learn("MIGROS RENENS", "groceries");
        tagger.learn("GRIMPER.CH LAUSANNE", "climbing");

        let stats = tagger.stats();
        assert_eq!(stats.len(), 2);
        let groceries = stats.iter().find(|s| s.tag == "groceries").unwrap();
        assert_eq!(groceries.trained_docs, 2);
        assert_eq!(groceries.exact_keys, 2);
        let climbing = stats.iter().find(|s| s.tag == "climbing").unwrap();
        assert_eq!(climbing.trained_docs, 1);
        assert_eq!(climbing.exact_keys, 1);
    }

    #[test]
    fn roundtrips_through_json() {
        let mut tagger = Tagger::with_min_confidence(0.5);
        tagger.learn("GRIMPER.CH LAUSANNE", "climbing");
        let restored = Tagger::from_json(&tagger.to_json().unwrap()).unwrap();
        assert_eq!(restored.suggest("GRIMPER.CH LAUSANNE").unwrap().tag, "climbing");
        assert_eq!(restored.min_confidence, 0.5);
    }
}
