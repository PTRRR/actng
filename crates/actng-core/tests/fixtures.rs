//! Data-driven suite over the vendored `testdata/bank2ynab/` exports (SPEC §6,
//! §7 task 1). Each fixture has an expectation: how many entries it should
//! yield, and how many of those should carry a parsed date/amount. Fixtures
//! the importer can't handle yet are `known_failing: true` — the assertion
//! flips (fail the row, not the fixture) so a hardening step (SPEC §4.2) is
//! done exactly when it lets that fixture's `known_failing` flip to `false`.

use actng_core::read_entries_from_path;
use std::path::{Path, PathBuf};

struct Expectation {
    file: &'static str,
    min_entries: usize,
    min_with_date: usize,
    min_with_amount: usize,
    known_failing: bool,
}

const EXPECTATIONS: &[Expectation] = &[
    // Already handled by the current importer.
    Expectation { file: "20171001-20171106.csv", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },
    Expectation { file: "2018-04-14_10-52-13_bunq-transactieoverzicht.csv", min_entries: 3, min_with_date: 3, min_with_amount: 3, known_failing: false },
    Expectation { file: "20180226_12345678.csv", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },
    Expectation { file: "2019-03-02_11-50-46_bunq-statement.csv", min_entries: 7, min_with_date: 7, min_with_amount: 7, known_failing: false },

    // Fixed by encoding fallback (iso-8859-1 / cp1250, SPEC §4.2 task 2).
    Expectation { file: "CSV_A_20180414_112204.csv", min_entries: 3, min_with_date: 3, min_with_amount: 3, known_failing: false },
    Expectation { file: "Movements_1234512345_201805271848.csv", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },
    Expectation { file: "TH_20180101-20180523_page_1.csv", min_entries: 9, min_with_date: 9, min_with_amount: 9, known_failing: false },
    Expectation { file: "TH_20180101-20180523_strana_1.csv", min_entries: 9, min_with_date: 9, min_with_amount: 9, known_failing: false },
    Expectation { file: "umsatz-1234________1234-20180227.CSV", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },

    // Fixed by delimiter sniffing (Polish/Czech/Belgian `;`, tab; SPEC §4.2 task 2).
    Expectation { file: "20180226-1000594757-umsatz.CSV", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },
    Expectation { file: "Lista_operacji_20240920_012205.csv", min_entries: 5, min_with_date: 5, min_with_amount: 5, known_failing: false },
    Expectation { file: "TH_20180521-20180523.csv", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },
    Expectation { file: "Umsaetze_KtoNr538917600_EUR_02-03-2018_1503.CSV", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },
    Expectation { file: "export_BE11123456789012_20180304_1422.csv", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },
    Expectation { file: "test_raiffeisen_01.csv", min_entries: 73, min_with_date: 73, min_with_amount: 73, known_failing: false },
    Expectation { file: "test_raiffeisen_02.csv", min_entries: 73, min_with_date: 73, min_with_amount: 73, known_failing: false },
    Expectation { file: "test_regex_123456_87654321_1234_21_12.csv", min_entries: 73, min_with_date: 73, min_with_amount: 73, known_failing: false },
    Expectation { file: "test_row_format_neg_inflow.csv", min_entries: 73, min_with_date: 73, min_with_amount: 73, known_failing: false },
    Expectation { file: "test_row_format_CD_flag.csv", min_entries: 73, min_with_date: 73, min_with_amount: 73, known_failing: false },
    // 2 trailing footer rows ("FOOTER ROW TEST") share the data rows' field
    // count so they survive delimiter sniffing; they correctly get no
    // date/amount rather than failing the row.
    Expectation { file: "test_headers.csv", min_entries: 75, min_with_date: 73, min_with_amount: 73, known_failing: false },

    // Fixed by preamble skipping + split debit/credit columns (SPEC §4.2
    // task 3). Each real transaction here is followed by a continuation row
    // (extra description text, no date/amount of its own) — that's not a
    // failure, just a second entry with nothing to tag.
    Expectation { file: "TransactionHistory_20180418043121.csv", min_entries: 16, min_with_date: 8, min_with_amount: 8, known_failing: false },

    // Fixed by split debit/credit columns (SPEC §4.2 task 3): the amount
    // lands in one of two columns depending on debit vs. credit, so the
    // single-amount-column detection used to leave the other side blank.
    Expectation { file: "test_BOI_TransactionExport.csv", min_entries: 27, min_with_date: 27, min_with_amount: 27, known_failing: false },
    Expectation { file: "export_KBC-Mastercard Business Essential_20200204_1604.csv", min_entries: 3, min_with_date: 3, min_with_amount: 3, known_failing: false },

    // Still needs split debit/credit, but these two are headerless: with no
    // header text to match "debit"/"credit" hints against, detection has no
    // way to tell the two amount columns apart. Not solvable without a
    // content-based split heuristic, which isn't in scope yet.
    Expectation { file: "test_delimiter_tab.csv", min_entries: 73, min_with_date: 73, min_with_amount: 73, known_failing: true },
    Expectation { file: "test_row_format_default.csv", min_entries: 73, min_with_date: 73, min_with_amount: 73, known_failing: true },

    // Fixed by date/amount format tolerance (SPEC §4.2 task 4): currency
    // symbols in amounts, and %d %b %Y / %d %b %y / %d-%b-%Y / %Y.%m.%d.
    // date formats (plus recognizing month-name dates by content, not just
    // header hints, so headerless files pick up a date column at all).
    Expectation { file: "MS_JANE_SMITH_01-12-2019_14-12-2019.csv", min_entries: 13, min_with_date: 10, min_with_amount: 13, known_failing: false },
    Expectation { file: "W80844_EBH_201945.202122.csv", min_entries: 2, min_with_date: 2, min_with_amount: 2, known_failing: false },
    Expectation { file: "dba33fceecd62c3c727893361e0ba4d3.P000000027355791.csv", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },
    Expectation { file: "statement_1.csv", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: false },

    // Not called out by any SPEC §4.2 task; tracked so it doesn't regress
    // silently and stays a candidate for a future date-format addition.
    Expectation { file: "MonzoDataExport_February2018_2018-02-26_174335.csv", min_entries: 1, min_with_date: 1, min_with_amount: 1, known_failing: true },
];

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/bank2ynab")
}

fn meets_expectations(exp: &Expectation, path: &Path) -> bool {
    let Ok(import) = read_entries_from_path(path, None) else { return false };
    let entries = &import.entries;
    entries.len() >= exp.min_entries
        && entries.iter().filter(|e| e.date.is_some()).count() >= exp.min_with_date
        && entries.iter().filter(|e| e.amount.is_some()).count() >= exp.min_with_amount
        && entries.iter().all(|e| !e.description.trim().is_empty())
}

#[test]
fn fixture_expectations() {
    let dir = fixtures_dir();
    let mut failures = Vec::new();
    for exp in EXPECTATIONS {
        let path = dir.join(exp.file);
        assert!(path.exists(), "fixture listed in expectations table not found on disk: {}", exp.file);
        let meets = meets_expectations(exp, &path);
        if exp.known_failing && meets {
            failures.push(format!("{}: now meets its expectations, flip `known_failing` to false", exp.file));
        } else if !exp.known_failing && !meets {
            failures.push(format!("{}: does not meet its expectations", exp.file));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Guards against the table silently drifting from `testdata/bank2ynab/` as
/// fixtures are added or renamed.
#[test]
fn expectations_cover_every_fixture() {
    let dir = fixtures_dir();
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.to_lowercase().ends_with(".csv"))
        .collect();
    files.sort();

    let mut expected: Vec<String> = EXPECTATIONS.iter().map(|e| e.file.to_string()).collect();
    expected.sort();

    assert_eq!(files, expected, "testdata/bank2ynab contents drifted from the fixtures.rs expectations table");
}
