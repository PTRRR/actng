use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::entry::Entry;
use crate::error::Error;
use crate::import::{decode_text, looks_delimited, read_entries_from_path_with_layouts, Import};
use crate::normalize::normalize;
use crate::profile::Profile;

/// How many leading bytes of a `.txt` file to read when sniffing whether it
/// looks like a delimited bank export rather than free-form prose.
const SNIFF_BYTES: usize = 8192;

/// Result of a single file import during a batch run.
#[derive(Debug)]
pub struct FileImport {
    pub path: PathBuf,
    pub result: Result<Import, Error>,
}

/// Find all files in `path` (recursively, skipping hidden directories) that
/// look like bank statements.
pub fn discover(path: impl AsRef<Path>) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    let walk = fs::read_dir(path)?;

    for entry in walk {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let hidden = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
            if hidden {
                continue;
            }
            files.extend(discover(&path)?);
        } else if is_bank_file(&path) {
            files.push(path);
        }
    }
    Ok(files)
}

/// `.csv`/`.tsv` are accepted by extension alone; `.txt` is only accepted if
/// its content sniffs as a delimited table rather than free-form prose (a
/// stray `notes.txt` shouldn't be imported and reported as a parse error).
fn is_bank_file(path: &Path) -> bool {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "csv" | "tsv" => true,
        "txt" => sniffs_as_delimited(path),
        _ => false,
    }
}

fn sniffs_as_delimited(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else { return false };
    let mut buf = vec![0u8; SNIFF_BYTES];
    let Ok(n) = file.read(&mut buf) else { return false };
    buf.truncate(n);
    let (text, _) = decode_text(&buf);
    looks_delimited(&text)
}

/// Batch import all files in a directory, reusing layout memory from a `Profile`.
pub fn import_dir(path: impl AsRef<Path>, profile: &Profile) -> Result<Vec<FileImport>, Error> {
    let files = discover(path)?;
    let mut results = Vec::new();

    for file in files {
        let result = read_entries_from_path_with_layouts(&file, &profile.layouts);
        results.push(FileImport { path: file, result });
    }

    Ok(results)
}

/// The owned, deduplicated result of importing a whole directory: every
/// entry that survived dedup, which source file it came from, the files
/// that failed to import, and how many duplicates were dropped.
///
/// Unlike a borrowed `&[FileImport]`, a `Dataset` can be held onto and
/// re-queried (re-suggest after every training step, write an export) once
/// the originating `Import`s have been consumed.
#[derive(Debug)]
pub struct Dataset {
    /// Deduplicated entries, in file order.
    pub entries: Vec<Entry>,
    /// Parallel to `entries`: index into `sources` for where it came from.
    pub source: Vec<usize>,
    /// Files that contributed at least one entry.
    pub sources: Vec<PathBuf>,
    /// Files that failed to import, with the reason.
    pub failures: Vec<(PathBuf, Error)>,
    pub duplicates_dropped: usize,
}

/// A dedup key stable across dates and card numbers for the same merchant:
/// (date, normalized description key, amount in integer cents).
type DedupKey = (Option<chrono::NaiveDate>, String, Option<i64>);

fn dedup_key(entry: &Entry) -> DedupKey {
    (entry.date, normalize(&entry.description).key, entry.amount.map(|a| (a * 100.0).round() as i64))
}

/// Consume a batch of `FileImport`s into a single deduplicated `Dataset`.
/// Per-file failures are collected rather than dropped; entries across all
/// successful files are deduplicated together (see `dedup_key`).
pub fn collect(imports: Vec<FileImport>) -> Dataset {
    use std::collections::HashSet;

    let mut entries = Vec::new();
    let mut source = Vec::new();
    let mut sources = Vec::new();
    let mut failures = Vec::new();
    let mut seen = HashSet::new();
    let mut dropped = 0;

    for imp in imports {
        let FileImport { path, result } = imp;
        match result {
            Ok(import) => {
                let idx = sources.len();
                sources.push(path);
                for entry in import.entries {
                    if seen.insert(dedup_key(&entry)) {
                        entries.push(entry);
                        source.push(idx);
                    } else {
                        dropped += 1;
                    }
                }
            }
            Err(e) => failures.push((path, e)),
        }
    }

    Dataset { entries, source, sources, failures, duplicates_dropped: dropped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::ImportProfile;
    use crate::profile::Profile;
    use std::fs;
    use tempfile::tempdir;

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

    fn test_import(entries: Vec<crate::entry::Entry>, fingerprint: &str) -> crate::import::Import {
        crate::import::Import {
            profile: test_import_profile(),
            entries,
            fingerprint: fingerprint.to_string(),
            delimiter: b',',
            encoding: crate::import::Encoding::Utf8,
            skipped_rows: 0,
        }
    }

    #[test]
    fn discover_finds_bank_files_recursively_and_skips_prose_txt_and_hidden_dirs() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("statement1.csv"), "a,b,c").unwrap();
        fs::write(
            root.join("notes.txt"),
            "Meeting notes:\nDiscussed budget, timeline, and resources.\nFollow up next week.\n",
        )
        .unwrap();
        fs::write(
            root.join("export.txt"),
            "01.02.2025\tCOOP LAUSANNE\t-12.50\n03.02.2025\tMIGROS RENENS\t-8.90\n",
        )
        .unwrap();

        let sub = root.join("bank");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("statement2.tsv"), "a\tb\tc").unwrap();
        fs::write(sub.join("image.png"), "binary").unwrap();

        let hidden = root.join(".git");
        fs::create_dir(&hidden).unwrap();
        fs::write(hidden.join("config.csv"), "a,b,c").unwrap();

        let found = discover(root).unwrap();
        let paths: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_str().unwrap()).collect();

        assert!(paths.contains(&"statement1.csv"));
        assert!(paths.contains(&"statement2.tsv"));
        assert!(paths.contains(&"export.txt"), "tab-delimited .txt should sniff as a bank file");
        assert!(!paths.contains(&"notes.txt"), "prose .txt should not sniff as a bank file");
        assert!(!paths.contains(&"config.csv"), "hidden directories are skipped");
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn import_dir_uses_layouts() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        let csv_content = "Date,Desc,Amount\n2025-01-01,Test,-10.00";
        let path = root.join("test.csv");
        fs::write(&path, csv_content).unwrap();

        // Create a profile with a pre-existing layout for this file
        let mut profile = Profile::new("test");
        let fresh_import = crate::import::read_entries_from_path(&path, None).unwrap();
        profile.layouts.insert(fresh_import.fingerprint.clone(), fresh_import.profile.clone());

        let results = import_dir(root, &profile).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].result.is_ok());
    }

    #[test]
    fn collect_deduplicates_entries_across_files() {
        use crate::entry::Entry;
        use chrono::NaiveDate;

        let date = NaiveDate::from_ymd_opt(2025, 1, 1);
        let make_entry = |desc: &str, amount: f64| Entry {
            date,
            description: desc.to_string(),
            amount: Some(amount),
            raw: vec![],
        };

        let import_a =
            test_import(vec![make_entry("COOP LAUSANNE 123", -12.50), make_entry("MIGROS", -8.00)], "a");
        // Same merchant/date/amount as one entry in file a: a cross-file duplicate.
        let import_b = test_import(vec![make_entry("COOP LAUSANNE 456", -12.50)], "b");

        let imports = vec![
            FileImport { path: PathBuf::from("a.csv"), result: Ok(import_a) },
            FileImport { path: PathBuf::from("b.csv"), result: Ok(import_b) },
        ];

        let dataset = collect(imports);
        assert_eq!(dataset.entries.len(), 2, "cross-file duplicate should be dropped");
        assert_eq!(dataset.duplicates_dropped, 1);
        assert_eq!(dataset.sources, vec![PathBuf::from("a.csv"), PathBuf::from("b.csv")]);
        assert_eq!(dataset.source, vec![0, 0], "both surviving entries came from file a");
        assert!(dataset.failures.is_empty());
    }

    #[test]
    fn collect_reports_per_file_failures_without_losing_successes() {
        use crate::entry::Entry;
        use chrono::NaiveDate;

        let good = test_import(
            vec![Entry {
                date: NaiveDate::from_ymd_opt(2025, 1, 1),
                description: "COOP".to_string(),
                amount: Some(-5.0),
                raw: vec![],
            }],
            "good",
        );

        let imports = vec![
            FileImport { path: PathBuf::from("good.csv"), result: Ok(good) },
            FileImport { path: PathBuf::from("broken.csv"), result: Err(Error::Empty) },
        ];

        let dataset = collect(imports);
        assert_eq!(dataset.entries.len(), 1);
        assert_eq!(dataset.sources, vec![PathBuf::from("good.csv")]);
        assert_eq!(dataset.failures.len(), 1);
        assert_eq!(dataset.failures[0].0, PathBuf::from("broken.csv"));
    }
}
