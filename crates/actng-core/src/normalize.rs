use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// The noise-filtered form of an entry description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Normalized {
    /// Informative tokens, in text order (IBANs and phone numbers appended).
    pub tokens: Vec<String>,
    /// Order-independent key over the distinct tokens; two descriptions that
    /// differ only in noise (dates, card numbers, references) share a key.
    pub key: String,
    /// IBANs found in the text — stable counterparty identifiers.
    pub ibans: Vec<String>,
    /// Phone numbers found in the text (e.g. TWINT P2P recipients).
    pub phones: Vec<String>,
}

static RE_IBAN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[a-z]{2}\d{2}[a-z0-9]{10,30}\b").unwrap());
static RE_PHONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+\d{7,15}\b").unwrap());
static RE_CARD_MASK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bx{2,}\d{2,8}\b").unwrap());
static RE_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{1,4}[./-]\d{1,2}[./-]\d{1,4}\b").unwrap());
static RE_DECIMAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d+[.,]\d+\b").unwrap());
static RE_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z0-9]+").unwrap());

/// Function words and legal-form suffixes that carry no signal for tagging.
const STOPWORDS: &[&str] = &[
    "du", "de", "des", "la", "le", "les", "un", "une", "et", "au", "aux", "en", "sur", "pour",
    "dans", "der", "die", "das", "und", "im", "am", "the", "of", "and", "no", "nr", "sa", "ag",
    "sarl", "gmbh",
];

/// Lowercase and strip diacritics: `DÉBIT` -> `debit`, `ZÜRICH` -> `zurich`.
fn fold(text: &str) -> String {
    text.nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

fn digit_fraction(token: &str) -> f64 {
    let digits = token.chars().filter(|c| c.is_ascii_digit()).count();
    digits as f64 / token.chars().count() as f64
}

/// Strip noise from a raw entry description and split it into tokens.
///
/// Removed as noise: dates, masked card numbers (`XXXX0918`), decimal amounts,
/// and mostly-numeric tokens (references, payment IDs, postal codes). IBANs
/// and phone numbers are pulled out first and kept both as features and as
/// tokens, since they identify counterparties.
pub fn normalize(text: &str) -> Normalized {
    let mut text = fold(text);

    let ibans: Vec<String> = RE_IBAN.find_iter(&text).map(|m| m.as_str().to_string()).collect();
    text = RE_IBAN.replace_all(&text, " ").into_owned();
    let phones: Vec<String> = RE_PHONE.find_iter(&text).map(|m| m.as_str().to_string()).collect();
    text = RE_PHONE.replace_all(&text, " ").into_owned();

    text = RE_CARD_MASK.replace_all(&text, " ").into_owned();
    text = RE_DATE.replace_all(&text, " ").into_owned();
    text = RE_DECIMAL.replace_all(&text, " ").into_owned();

    let mut tokens: Vec<String> = RE_WORD
        .find_iter(&text)
        .map(|m| m.as_str().to_string())
        .filter(|t| t.chars().count() >= 2)
        .filter(|t| digit_fraction(t) <= 0.5)
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .collect();
    tokens.extend(ibans.iter().cloned());
    tokens.extend(phones.iter().cloned());

    let key = tokens.iter().collect::<BTreeSet<_>>().into_iter().cloned().collect::<Vec<_>>().join(" ");

    Normalized { tokens, key, ibans, phones }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_dates_cards_and_references() {
        let n = normalize(
            "ACHAT/SHOPPING EN LIGNE DU 05.12.2025 USD 21.62 AU COURS DE 0.8112 MONTANT DANS LA \
             MONNAIE DU COMPTE 17.54 1.5% FRAIS DE TRAITEMENT CHF 0.26 CARTE N° XXXX0918 \
             CLAUDE.AI SUBSCRIPTION SAN FRANCISCO",
        );
        assert!(n.tokens.contains(&"claude".to_string()));
        assert!(n.tokens.contains(&"subscription".to_string()));
        assert!(!n.tokens.iter().any(|t| t.contains("0918")));
        assert!(!n.tokens.iter().any(|t| t.chars().all(|c| c.is_ascii_digit())));
    }

    #[test]
    fn extracts_ibans_and_phones() {
        let n = normalize(
            "DÉBIT UBS SWITZERLAND AG BAHNHOFSTRASSE 45 8098 ZÜRICH CH893000520211491010B SERAFE \
             AG REFERENCE DE L'EXPEDITEUR: 20251213000800685497209",
        );
        assert_eq!(n.ibans, vec!["ch893000520211491010b"]);
        assert!(n.tokens.contains(&"serafe".to_string()));
        assert!(!n.tokens.iter().any(|t| t == "20251213000800685497209"));

        let p = normalize("ENVOI D'ARGENT TWINT DU 01.12.2025 POUR NUMÉRO MOBILE. +41774042694 MATOS, SEBASTIEN");
        assert_eq!(p.phones, vec!["+41774042694"]);
        assert!(p.tokens.contains(&"sebastien".to_string()));
    }

    #[test]
    fn key_is_stable_across_dates() {
        let a = normalize("ACHAT/PRESTATION TWINT DU 06.11.2025 GRIMPER.CH LAUSANNE (CH)");
        let b = normalize("ACHAT/PRESTATION TWINT DU 21.10.2025 GRIMPER.CH LAUSANNE (CH)");
        assert_eq!(a.key, b.key);
        assert!(!a.key.is_empty());
    }

    #[test]
    fn folds_accents() {
        let n = normalize("AIMÉ POULY ZÜRICH FRANÇAIS");
        assert_eq!(n.tokens, vec!["aime", "pouly", "zurich", "francais"]);
    }
}
