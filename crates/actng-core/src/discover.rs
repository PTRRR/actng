use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::import::{read_entries_from_path_with_layouts, Import};
use crate::profile::Profile;

/// Result of a single file import during a batch run.
#[derive(Debug)]
pub struct FileImport {
    pub path: PathBuf,
    pub result: Result<Import, Error>,
}

/// Find all files in `path` (recursively) that look like bank statements.
pub fn discover(path: impl AsRef<Path>) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    let walk = fs::read_dir(path)?;

    for entry in walk {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            files.extend(discover(&path)?);
        } else if is_bank_file(&path) {
            files.push(path);
        }
    }
    Ok(files)
}

fn is_bank_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "csv" | "txt" | "tsv" => true,
        _ => false,
    }
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

/// Deduplicate entries across multiple imports.
///
/// Two entries are considered duplicates if they share the same
/// (date, normalized description, amount).
pub fn deduplicate(entries: Vec<crate::entry::Entry>) -> (Vec<crate::entry::Entry>, usize) {
    use crate::normalize::normalize;
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    let mut dropped = 0;

    for entry in entries {
        let key = (
            entry.date,
            normalize(&entry.description).key,
            entry.amount.map(|a| (a * 100.0).round() as i64),
        );
        if seen.insert(key) {
            unique.push(entry);
        } else {
            dropped += 1;
        }
    }
    (unique, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discover_finds_bank_files_recursively() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("statement1.csv"), "a,b,c").unwrap();
        fs::write(root.join("notes.txt"), "hello").unwrap();

        let sub = root.join("bank");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("statement2.tsv"), "a\tb\tc").unwrap();
        fs::write(sub.join("image.png"), "binary").unwrap();

        let found = discover(root).unwrap();
        assert_eq!(found.len(), 3);
        let paths: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(paths.contains(&"statement1.csv"));
        assert!(paths.contains(&"notes.txt"));
        assert!(paths.contains(&"statement2.tsv"));
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
        profile.layouts.insert(
            fresh_import.fingerprint.clone(),
            fresh_import.profile.clone(),
        );

        let results = import_dir(root, &profile).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].result.is_ok());
    }

    #[test]
    fn deduplicate_removes_identical_entries() {
        use crate::entry::Entry;
        use chrono::NaiveDate;

        let date = NaiveDate::from_ymd_opt(2025, 1, 1);
        let entries = vec![
            Entry {
                date,
                description: "COOP LAUSANNE 123".to_string(),
                amount: Some(-12.50),
                raw: vec![],
            },
            Entry {
                date,
                description: "COOP LAUSANNE 456".to_string(), // different raw/desc but same normalized
                amount: Some(-12.50),
                raw: vec![],
            },
            Entry {
                date,
                description: "MIGROS".to_string(),
                amount: Some(-8.00),
                raw: vec![],
            },
        ];

        let (unique, dropped) = deduplicate(entries);
        assert_eq!(unique.len(), 2);
        assert_eq!(dropped, 1);
    }
}
