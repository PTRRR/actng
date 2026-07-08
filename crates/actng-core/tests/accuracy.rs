//! Tagger accuracy benchmark (SPEC §6, §7 task 5) over synthetic labeled
//! transactions (`testdata/sparkov/`, see its README for provenance).
//!
//! The split is by *merchant*, not by row, so it exercises both paths the
//! design relies on: 1/5 of distinct merchants are held out of training
//! entirely (novel to the tagger) to check Bayes/abstention behavior on
//! unseen merchants; the rest keep some occurrences for training and some
//! for testing, so exact-match recall (the common case for a real bank
//! export re-tagging the same merchant over time) gets checked too. Split is
//! index-based, not RNG-based, so it's deterministic without a dependency.

use actng_core::Tagger;
use std::collections::HashSet;
use std::path::Path;

fn load_rows() -> Vec<(String, String)> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/sparkov/transactions.csv");
    let mut reader = csv::Reader::from_path(path).unwrap();
    reader
        .records()
        .map(|r| {
            let r = r.unwrap();
            (r[0].to_string(), r[1].to_string())
        })
        .collect()
}

struct Split<'a> {
    train: Vec<&'a (String, String)>,
    test_known: Vec<&'a (String, String)>,
    test_novel: Vec<&'a (String, String)>,
}

fn train_test_split(rows: &[(String, String)]) -> Split<'_> {
    let mut merchants: Vec<&str> = rows.iter().map(|(d, _)| d.as_str()).collect();
    merchants.sort_unstable();
    merchants.dedup();
    let held_out: HashSet<&str> =
        merchants.iter().enumerate().filter(|(i, _)| i % 5 == 0).map(|(_, d)| *d).collect();

    let mut split = Split { train: Vec::new(), test_known: Vec::new(), test_novel: Vec::new() };
    let mut i = 0usize;
    for row in rows {
        if held_out.contains(row.0.as_str()) {
            split.test_novel.push(row);
        } else if i.is_multiple_of(5) {
            split.test_known.push(row);
            i += 1;
        } else {
            split.train.push(row);
            i += 1;
        }
    }
    split
}

#[test]
fn bayes_accuracy_floor() {
    let rows = load_rows();
    assert!(rows.len() > 1000, "fixture unexpectedly small: {} rows", rows.len());
    let split = train_test_split(&rows);
    assert!(split.test_novel.len() > 100, "too few held-out merchants to test abstention");

    let mut tagger = Tagger::new();
    for (description, category) in &split.train {
        tagger.learn(description, category);
    }

    // Known merchants: exact-match should dominate and be almost always
    // right (last-confirmation-wins noise from the handful of merchant
    // names the generator reused across categories is the only expected
    // miss — see SPEC §3's caveat on normalized-key collisions).
    let (correct, incorrect, abstained) = tally(&tagger, &split.test_known);
    let answered = correct + incorrect;
    let accuracy = correct as f64 / answered as f64;
    println!(
        "known merchants: n={} correct={correct} incorrect={incorrect} abstained={abstained} accuracy={accuracy:.3}",
        split.test_known.len()
    );
    assert!(accuracy >= 0.85, "known-merchant accuracy floor breached: {accuracy:.3}");
    assert!(
        answered * 2 >= split.test_known.len(),
        "too many abstentions on known merchants: only {answered}/{} answered",
        split.test_known.len()
    );

    // Novel merchants: never learned, so tokens are almost entirely unseen.
    // The tagger should mostly decline rather than guess. It's not perfect —
    // fake company names share generic surname tokens by chance, which
    // occasionally pushes an unrelated category over the confidence
    // threshold — so this checks the abstention *rate*, not the accuracy of
    // the rare confident guess.
    let (correct, incorrect, abstained) = tally(&tagger, &split.test_novel);
    let abstention_rate = abstained as f64 / split.test_novel.len() as f64;
    println!(
        "novel merchants: n={} correct={correct} incorrect={incorrect} abstained={abstained} abstention_rate={abstention_rate:.3}",
        split.test_novel.len()
    );
    assert!(
        abstention_rate >= 0.8,
        "novel merchants should mostly abstain, got abstention rate {abstention_rate:.3}"
    );
}

fn tally(tagger: &Tagger, rows: &[&(String, String)]) -> (usize, usize, usize) {
    let (mut correct, mut incorrect, mut abstained) = (0, 0, 0);
    for (description, expected) in rows {
        match tagger.suggest(description) {
            Some(s) if &s.tag == expected => correct += 1,
            Some(_) => incorrect += 1,
            None => abstained += 1,
        }
    }
    (correct, incorrect, abstained)
}
