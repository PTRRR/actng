use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Incremental multinomial Naive Bayes classifier over token sets.
///
/// Tokens are deduplicated per document (set-of-words, as in spam filters and
/// GnuCash's import matcher) so a repeated token can't dominate a single entry.
/// Training is incremental: every confirmed tag updates the counts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NaiveBayes {
    /// Documents seen per tag (the prior).
    doc_counts: HashMap<String, u64>,
    /// tag -> token -> occurrence count.
    token_counts: HashMap<String, HashMap<String, u64>>,
    /// Total token occurrences per tag (likelihood denominator).
    tag_token_totals: HashMap<String, u64>,
    vocab: HashSet<String>,
    total_docs: u64,
}

impl NaiveBayes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one document's tokens under `tag`.
    pub fn train(&mut self, tokens: &[String], tag: &str) {
        let distinct: BTreeSet<&str> = tokens.iter().map(String::as_str).collect();
        if distinct.is_empty() {
            return;
        }
        *self.doc_counts.entry(tag.to_string()).or_default() += 1;
        self.total_docs += 1;
        let counts = self.token_counts.entry(tag.to_string()).or_default();
        for token in &distinct {
            *counts.entry(token.to_string()).or_default() += 1;
            self.vocab.insert(token.to_string());
        }
        *self.tag_token_totals.entry(tag.to_string()).or_default() += distinct.len() as u64;
    }

    /// Posterior probability per tag, sorted most likely first.
    ///
    /// Tokens outside the training vocabulary are ignored; if no token is
    /// known (or the model is untrained) the result is empty — no evidence,
    /// no guess.
    pub fn classify(&self, tokens: &[String]) -> Vec<(String, f64)> {
        if self.total_docs == 0 {
            return Vec::new();
        }
        let known: BTreeSet<&str> = tokens
            .iter()
            .map(String::as_str)
            .filter(|t| self.vocab.contains(*t))
            .collect();
        if known.is_empty() {
            return Vec::new();
        }

        let vocab_size = self.vocab.len() as f64;
        let mut scores: Vec<(String, f64)> = self
            .doc_counts
            .iter()
            .map(|(tag, &docs)| {
                let mut log_p = (docs as f64 / self.total_docs as f64).ln();
                let counts = self.token_counts.get(tag);
                let total = self.tag_token_totals.get(tag).copied().unwrap_or(0) as f64;
                for token in &known {
                    let c = counts.and_then(|m| m.get(*token)).copied().unwrap_or(0) as f64;
                    log_p += ((c + 1.0) / (total + vocab_size)).ln();
                }
                (tag.clone(), log_p)
            })
            .collect();

        // Log-sum-exp softmax to turn log scores into posteriors.
        let max = scores.iter().map(|(_, s)| *s).fold(f64::NEG_INFINITY, f64::max);
        let sum: f64 = scores.iter().map(|(_, s)| (s - max).exp()).sum();
        for (_, s) in &mut scores {
            *s = (*s - max).exp() / sum;
        }
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scores
    }

    /// Tags this model has seen at least once.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.doc_counts.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.total_docs == 0
    }

    /// Number of documents trained under `tag`, or 0 if it has none.
    pub fn doc_count(&self, tag: &str) -> u64 {
        self.doc_counts.get(tag).copied().unwrap_or(0)
    }

    /// Reverse one `train` call: decrement the doc and token counts recorded
    /// for `tokens` under `tag` (saturating at zero). If `tag`'s document
    /// count reaches zero it disappears entirely, same as `remove_tag`.
    pub fn untrain(&mut self, tokens: &[String], tag: &str) {
        let distinct: BTreeSet<&str> = tokens.iter().map(String::as_str).collect();
        if distinct.is_empty() || !self.doc_counts.contains_key(tag) {
            return;
        }
        let docs = self.doc_counts.entry(tag.to_string()).or_default();
        *docs = docs.saturating_sub(1);
        self.total_docs = self.total_docs.saturating_sub(1);
        if let Some(counts) = self.token_counts.get_mut(tag) {
            for token in &distinct {
                if let Some(c) = counts.get_mut(*token) {
                    *c = c.saturating_sub(1);
                    if *c == 0 {
                        counts.remove(*token);
                    }
                }
            }
        }
        if let Some(total) = self.tag_token_totals.get_mut(tag) {
            *total = total.saturating_sub(distinct.len() as u64);
        }
        if self.doc_counts.get(tag).copied().unwrap_or(0) == 0 {
            self.doc_counts.remove(tag);
            self.token_counts.remove(tag);
            self.tag_token_totals.remove(tag);
        }
        self.vocab = self.token_counts.values().flat_map(|m| m.keys().cloned()).collect();
    }

    /// Rewrite all training data recorded under `old` to `new`, merging into
    /// `new`'s counts if it already has training data of its own.
    pub fn rename_tag(&mut self, old: &str, new: &str) {
        if old == new {
            return;
        }
        if let Some(docs) = self.doc_counts.remove(old) {
            *self.doc_counts.entry(new.to_string()).or_default() += docs;
        }
        if let Some(counts) = self.token_counts.remove(old) {
            let entry = self.token_counts.entry(new.to_string()).or_default();
            for (token, count) in counts {
                *entry.entry(token).or_default() += count;
            }
        }
        if let Some(total) = self.tag_token_totals.remove(old) {
            *self.tag_token_totals.entry(new.to_string()).or_default() += total;
        }
    }

    /// Drop all training data recorded under `tag`.
    pub fn remove_tag(&mut self, tag: &str) {
        if let Some(docs) = self.doc_counts.remove(tag) {
            self.total_docs = self.total_docs.saturating_sub(docs);
        }
        self.token_counts.remove(tag);
        self.tag_token_totals.remove(tag);
        self.vocab = self.token_counts.values().flat_map(|m| m.keys().cloned()).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn classifies_by_shared_tokens() {
        let mut nb = NaiveBayes::new();
        nb.train(&toks("coop ls bel air lausanne"), "groceries");
        nb.train(&toks("migros silo renens"), "groceries");
        nb.train(&toks("grimper lausanne"), "climbing");

        let scores = nb.classify(&toks("coop ls city lausanne"));
        assert_eq!(scores[0].0, "groceries");
        assert!(scores[0].1 > 0.5);
        let total: f64 = scores.iter().map(|(_, p)| p).sum();
        assert!((total - 1.0).abs() < 1e-9);
    }

    #[test]
    fn no_evidence_means_no_guess() {
        let mut nb = NaiveBayes::new();
        assert!(nb.classify(&toks("anything")).is_empty());
        nb.train(&toks("coop lausanne"), "groceries");
        assert!(nb.classify(&toks("completely unseen words")).is_empty());
    }

    #[test]
    fn untrain_reverses_train() {
        let mut nb = NaiveBayes::new();
        nb.train(&toks("coop ls bel air lausanne"), "groceries");
        nb.train(&toks("migros silo renens"), "groceries");
        nb.train(&toks("grimper boulder"), "climbing");

        nb.untrain(&toks("grimper boulder"), "climbing");
        assert_eq!(nb.doc_count("climbing"), 0);
        assert!(!nb.tags().any(|t| t == "climbing"), "tag disappears once its doc count hits zero");
        assert!(nb.classify(&toks("grimper boulder")).is_empty(), "no lingering evidence for the untrained tag");

        // groceries still has one training doc left with correct counts.
        assert_eq!(nb.doc_count("groceries"), 2);
        nb.untrain(&toks("migros silo renens"), "groceries");
        assert_eq!(nb.doc_count("groceries"), 1);
        let scores = nb.classify(&toks("coop ls bel air lausanne"));
        assert_eq!(scores[0].0, "groceries");
    }

    #[test]
    fn untrain_on_unknown_tag_is_a_no_op() {
        let mut nb = NaiveBayes::new();
        nb.train(&toks("coop lausanne"), "groceries");
        nb.untrain(&toks("something else"), "unknown");
        assert_eq!(nb.doc_count("groceries"), 1);
        assert_eq!(nb.doc_count("unknown"), 0);
    }

    #[test]
    fn doc_count_tracks_training_volume() {
        let mut nb = NaiveBayes::new();
        assert_eq!(nb.doc_count("groceries"), 0);
        nb.train(&toks("coop lausanne"), "groceries");
        nb.train(&toks("migros renens"), "groceries");
        nb.train(&toks("grimper lausanne"), "climbing");
        assert_eq!(nb.doc_count("groceries"), 2);
        assert_eq!(nb.doc_count("climbing"), 1);
        assert_eq!(nb.doc_count("unknown"), 0);
    }
}
