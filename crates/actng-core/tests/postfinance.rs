//! End-to-end test against the real PostFinance export in `examples/`.

use std::path::PathBuf;

use actng_core::{normalize, read_entries_from_path, Source, Tagger};

fn example_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/export_mouvements_20260708_postfinance copy.csv")
}

#[test]
fn imports_the_postfinance_export() {
    let import = read_entries_from_path(example_path(), None).expect("import should succeed");

    let p = &import.profile;
    assert!(p.has_header);
    assert_eq!(p.date_column, Some(0), "first column is the date");
    assert_eq!(p.description_columns, vec![2], "notification text is the description");
    assert_eq!(p.amount_column, None, "this export format carries no amount column");

    assert!(import.entries.len() > 300, "got {} entries", import.entries.len());
    let dated = import.entries.iter().filter(|e| e.date.is_some()).count();
    assert_eq!(dated, import.entries.len(), "every row has a parseable date");
}

#[test]
fn normalization_strips_export_noise() {
    let import = read_entries_from_path(example_path(), None).unwrap();

    for entry in &import.entries {
        let n = normalize(&entry.description);
        assert!(!n.tokens.is_empty(), "no tokens for: {}", entry.description);
        for token in &n.tokens {
            assert!(!token.starts_with("xxxx"), "card mask survived: {token}");
            assert!(
                !token.chars().all(|c| c.is_ascii_digit()),
                "numeric noise survived: {token} in {}",
                entry.description
            );
        }
    }

    // Direct-debit entries expose the counterparty IBAN as a feature.
    let debit = import
        .entries
        .iter()
        .find(|e| e.description.contains("SERAFE"))
        .expect("SERAFE entry present");
    let n = normalize(&debit.description);
    assert!(n.ibans.contains(&"ch893000520211491010b".to_string()));
}

#[test]
fn learned_tags_apply_across_the_file() {
    let import = read_entries_from_path(example_path(), None).unwrap();
    let mut tagger = Tagger::with_min_confidence(0.5);

    // Bootstrap: tag one entry per merchant, as a user would in review.
    tagger.learn("ACHAT/PRESTATION TWINT DU 06.11.2025 GRIMPER.CH LAUSANNE (CH)", "climbing");
    tagger.learn(
        "ACHAT/SERVICE DU 07.06.2025 CARTE NO XXXX6273 COOP-5386 LS BEL AIR FOOBY LAUSANNE (CH)",
        "groceries",
    );
    tagger.learn("ACHAT/PRESTATION TWINT DU 04.12.2025 M MIGROS DU SILO RENENS VD (CH)", "groceries");
    tagger.learn(
        "ACHAT/SHOPPING EN LIGNE DU 02.11.2025 CARTE N° XXXX0918 INFOMANIAK.COM INTERNET",
        "it-services",
    );

    // Same merchant, different date and card: deterministic exact match.
    let exact = tagger
        .suggest("ACHAT/PRESTATION TWINT DU 21.10.2025 GRIMPER.CH LAUSANNE (CH)")
        .expect("grimper should match");
    assert_eq!(exact.tag, "climbing");
    assert_eq!(exact.source, Source::Exact);

    // Different COOP branch, never seen verbatim: Bayes generalizes.
    let bayes = tagger
        .suggest("ACHAT/PRESTATION TWINT DU 29.10.2025 COOP-5932 LS CITY ST.FRANÇ. K. LAUSANNE (CH)")
        .expect("coop city should get a suggestion");
    assert_eq!(bayes.tag, "groceries");
    assert_eq!(bayes.source, Source::Bayes);
    assert!(bayes.confidence > 0.5);

    // Unseen merchant with no shared evidence: review queue, not a guess.
    assert!(tagger.suggest("VIREMENT QUELCONQUE ZZZZZ").is_none());

    // Coverage over the whole file from just four learned entries.
    let suggested = import
        .entries
        .iter()
        .filter(|e| tagger.suggest(&e.description).is_some())
        .count();
    assert!(
        suggested > 60,
        "expected four merchants to cover a good chunk of the file, got {suggested}"
    );
}
