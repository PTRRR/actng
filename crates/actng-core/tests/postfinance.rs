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
}

#[test]
fn profile_run_partitions_results() {
    let import = read_entries_from_path(example_path(), None).unwrap();
    let mut profile = actng_core::Profile::new("test");

    // Learn some examples
    profile.learn("ACHAT/PRESTATION TWINT DU 06.11.2025 GRIMPER.CH LAUSANNE (CH)", "climbing");
    profile.learn("ACHAT/SERVICE DU 07.06.2025 CARTE NO XXXX6273 COOP-5386 LS BEL AIR FOOBY LAUSANNE (CH)", "groceries");

    let file_import = actng_core::FileImport { path: example_path(), result: Ok(import) };

    let dataset = actng_core::collect(vec![file_import]);
    let result = profile.run(&dataset);

    assert!(!result.tagged.is_empty(), "should have tagged entries");
    assert!(!result.review.is_empty(), "should have entries for review");
    assert_eq!(result.sources.len(), 1);
    assert_eq!(result.sources[0], example_path());
}

#[test]
fn profile_run_keeps_all_entries() {
    let import = read_entries_from_path(example_path(), None).unwrap();
    let profile = actng_core::Profile::new("test");

    let file_import_1 = actng_core::FileImport { path: example_path(), result: Ok(import.clone()) };
    let file_import_2 = actng_core::FileImport { path: example_path(), result: Ok(import.clone()) };

    let dataset = actng_core::collect(vec![file_import_1, file_import_2]);
    let result = profile.run(&dataset);

    let total_entries = result.tagged.len() + result.review.len();
    assert_eq!(total_entries, import.entries.len() * 2);
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
