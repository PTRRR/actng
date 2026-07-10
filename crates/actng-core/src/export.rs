use std::io::Write;

use crate::discover::Dataset;
use crate::error::Error;
use crate::profile::Profile;
use crate::tagger::Suggestion;

/// Per-category totals produced alongside the exported CSV.
///
/// Every entry contributes to exactly one bucket — its tag's category, or
/// `"uncategorized"` if it has no tag or its tag has no category — so the
/// totals sum to the same amount as the full dataset. Sorted by category
/// name.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub rows: usize,
    pub per_category: Vec<(String, f64)>,
}

/// Write the tagged dataset as CSV: `date, description, amount, tag,
/// category, source_file`. `suggestions` must be parallel to
/// `dataset.entries`; entries with `None` export with an empty tag and
/// category so nothing silently disappears from the file.
pub fn write_csv<W: Write>(
    w: W,
    dataset: &Dataset,
    profile: &Profile,
    suggestions: &[Option<Suggestion>],
) -> Result<Summary, Error> {
    debug_assert_eq!(dataset.entries.len(), suggestions.len());

    let mut wtr = csv::Writer::from_writer(w);
    wtr.write_record([
        "date",
        "description",
        "amount",
        "tag",
        "category",
        "source_file",
    ])?;

    let mut per_category: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    for (i, entry) in dataset.entries.iter().enumerate() {
        let suggestion = suggestions.get(i).and_then(|s| s.as_ref());
        let tag = suggestion.map(|s| s.tag.as_str()).unwrap_or("");
        let category = suggestion
            .and_then(|s| profile.tags.iter().find(|t| t.name == s.tag))
            .and_then(|t| t.category.as_deref())
            .unwrap_or("");

        let date = entry.date.map(|d| d.to_string()).unwrap_or_default();
        let amount = entry.amount.map(|a| a.to_string()).unwrap_or_default();
        let source_file = dataset
            .source
            .get(i)
            .and_then(|&idx| dataset.sources.get(idx))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        wtr.write_record([
            &date,
            &entry.description,
            &amount,
            tag,
            category,
            &source_file,
        ])?;

        let bucket = if category.is_empty() {
            "uncategorized"
        } else {
            category
        };
        *per_category.entry(bucket.to_string()).or_default() += entry.amount.unwrap_or(0.0);
    }

    wtr.flush()?;

    let mut per_category: Vec<(String, f64)> = per_category.into_iter().collect();
    per_category.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(Summary {
        rows: dataset.entries.len(),
        per_category,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::{collect, FileImport};
    use crate::entry::Entry;
    use crate::import::{Encoding, Import, ImportProfile};
    use crate::profile::Tag;
    use crate::tagger::Source;
    use chrono::NaiveDate;
    use std::path::PathBuf;

    fn test_import_profile() -> ImportProfile {
        ImportProfile {
            has_header: true,
            date_column: Some(0),
            description_columns: vec![1],
            amount_column: Some(2),
            debit_column: None,
            credit_column: None,
            date_formats: vec![],
        }
    }

    fn dataset_with(entries: Vec<Entry>, path: &str) -> Dataset {
        let import = Import {
            profile: test_import_profile(),
            entries,
            fingerprint: "fp".to_string(),
            delimiter: b',',
            encoding: Encoding::Utf8,
            skipped_rows: 0,
        };
        collect(vec![FileImport {
            path: PathBuf::from(path),
            result: Ok(import),
        }])
    }

    #[test]
    fn quotes_descriptions_with_commas_and_survives_round_trip() {
        let dataset = dataset_with(
            vec![Entry {
                date: NaiveDate::from_ymd_opt(2025, 1, 1),
                description: "COOP, LAUSANNE \"BEL AIR\"".to_string(),
                amount: Some(-12.50),
                raw: vec![],
            }],
            "coop.csv",
        );
        let profile = Profile::new("test");
        let suggestions = vec![Some(Suggestion {
            tag: "groceries".to_string(),
            confidence: 1.0,
            source: Source::Exact,
        })];

        let mut buf = Vec::new();
        write_csv(&mut buf, &dataset, &profile, &suggestions).unwrap();

        let mut rdr = csv::Reader::from_reader(buf.as_slice());
        let headers = rdr.headers().unwrap().clone();
        assert_eq!(
            headers,
            vec![
                "date",
                "description",
                "amount",
                "tag",
                "category",
                "source_file"
            ]
        );

        let record = rdr.records().next().unwrap().unwrap();
        assert_eq!(&record[1], "COOP, LAUSANNE \"BEL AIR\"");
        assert_eq!(&record[3], "groceries");
        assert_eq!(&record[5], "coop.csv");
    }

    #[test]
    fn untagged_entries_export_with_empty_tag_not_dropped() {
        let dataset = dataset_with(
            vec![Entry {
                date: NaiveDate::from_ymd_opt(2025, 1, 1),
                description: "MYSTERY MERCHANT".to_string(),
                amount: Some(-9.99),
                raw: vec![],
            }],
            "x.csv",
        );
        let profile = Profile::new("test");
        let suggestions = vec![None];

        let mut buf = Vec::new();
        let summary = write_csv(&mut buf, &dataset, &profile, &suggestions).unwrap();

        assert_eq!(summary.rows, 1);
        let mut rdr = csv::Reader::from_reader(buf.as_slice());
        let record = rdr.records().next().unwrap().unwrap();
        assert_eq!(
            &record[3], "",
            "untagged row keeps an empty tag, not dropped"
        );
    }

    #[test]
    fn overridden_entry_exports_with_its_pinned_tag_and_category() {
        let dataset = dataset_with(
            vec![Entry {
                date: NaiveDate::from_ymd_opt(2025, 1, 1),
                description: "COOP LAUSANNE".to_string(),
                amount: Some(-10.0),
                raw: vec![],
            }],
            "x.csv",
        );
        let mut profile = Profile::new("test");
        profile.tags.push(Tag { name: "gift".to_string(), category: Some("presents".to_string()), description: None });
        let entry = &dataset.entries[0];
        profile.set_override(entry, "gift");

        let suggestions: Vec<Option<Suggestion>> = dataset.entries.iter().map(|e| profile.suggest_entry(e)).collect();

        let mut buf = Vec::new();
        let summary = write_csv(&mut buf, &dataset, &profile, &suggestions).unwrap();

        let mut rdr = csv::Reader::from_reader(buf.as_slice());
        let record = rdr.records().next().unwrap().unwrap();
        assert_eq!(&record[3], "gift");
        assert_eq!(&record[4], "presents");
        assert_eq!(summary.per_category, vec![("presents".to_string(), -10.0)]);
    }

    #[test]
    fn per_category_totals_match_hand_computed_sums() {
        let dataset = dataset_with(
            vec![
                Entry {
                    date: None,
                    description: "A".to_string(),
                    amount: Some(-10.0),
                    raw: vec![],
                },
                Entry {
                    date: None,
                    description: "B".to_string(),
                    amount: Some(-5.0),
                    raw: vec![],
                },
                Entry {
                    date: None,
                    description: "C".to_string(),
                    amount: Some(-3.0),
                    raw: vec![],
                },
                Entry {
                    date: None,
                    description: "D".to_string(),
                    amount: Some(100.0),
                    raw: vec![],
                },
            ],
            "x.csv",
        );
        let mut profile = Profile::new("test");
        profile.tags.push(Tag {
            name: "groceries".to_string(),
            category: Some("living".to_string()),
            description: None,
        });
        profile.tags.push(Tag {
            name: "misc".to_string(),
            category: None,
            description: None,
        });

        let sugg = |tag: &str| {
            Some(Suggestion {
                tag: tag.to_string(),
                confidence: 1.0,
                source: Source::Exact,
            })
        };
        let suggestions = vec![sugg("groceries"), sugg("groceries"), sugg("misc"), None];

        let mut buf = Vec::new();
        let summary = write_csv(&mut buf, &dataset, &profile, &suggestions).unwrap();

        assert_eq!(
            summary.per_category,
            vec![
                ("living".to_string(), -15.0),
                ("uncategorized".to_string(), 97.0)
            ]
        );
    }
}
