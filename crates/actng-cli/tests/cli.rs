//! End-to-end tests running the `actng` binary in a temp dir, per SPEC.md §6.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn actng() -> Command {
    Command::cargo_bin("actng").unwrap()
}

const FIXTURE_CSV: &str = "Date,Description,Amount\n\
2025-01-01,COOP LAUSANNE,-12.50\n\
2025-01-02,MIGROS RENENS,-8.90\n\
2025-01-03,SALARY PAYMENT,2500.00\n";

fn write_fixture(dir: &Path) {
    std::fs::write(dir.join("statement.csv"), FIXTURE_CSV).unwrap();
}

#[test]
fn init_creates_profile_and_refuses_overwrite() {
    let dir = tempfile::tempdir().unwrap();

    actng()
        .current_dir(&dir)
        .args(["init", "--name", "test", "--tags", "groceries,rent"])
        .assert()
        .success();

    assert!(dir.path().join("actng.json").exists());

    actng()
        .current_dir(&dir)
        .args(["init", "--name", "test"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn scan_reports_layout_detail_for_discovered_files() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture(dir.path());

    actng()
        .current_dir(&dir)
        .args(["init", "--name", "test"])
        .assert()
        .success();

    actng()
        .current_dir(&dir)
        .arg("scan")
        .assert()
        .success()
        .stdout(predicate::str::contains("statement.csv"))
        .stdout(predicate::str::contains("detected"))
        .stdout(predicate::str::contains("utf-8"));
}

#[test]
fn tag_exits_2_and_prints_summary_when_review_queue_nonempty() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture(dir.path());

    actng()
        .current_dir(&dir)
        .args(["init", "--name", "test"])
        .assert()
        .success();

    actng()
        .current_dir(&dir)
        .arg("tag")
        .assert()
        .code(2)
        .stdout(predicate::str::contains("need review"));
}

#[test]
fn tag_exits_0_once_every_entry_is_confidently_tagged() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture(dir.path());

    actng()
        .current_dir(&dir)
        .args(["init", "--name", "test"])
        .assert()
        .success();

    // Simulate a completed review session by training the profile directly
    // (dialoguer prompts can't be scripted headlessly; see FIXES.md F10).
    let profile_path = dir.path().join("actng.json");
    let mut profile = actng_core::Profile::load(&profile_path).unwrap();
    profile.learn("COOP LAUSANNE", "groceries");
    profile.learn("MIGROS RENENS", "groceries");
    profile.learn("SALARY PAYMENT", "income");
    profile.save(&profile_path).unwrap();

    actng()
        .current_dir(&dir)
        .arg("tag")
        .assert()
        .success()
        .stdout(predicate::str::contains("0 need review"));
}

#[test]
fn tags_list_shows_trained_counts_and_rm_yes_removes_the_tag() {
    let dir = tempfile::tempdir().unwrap();
    actng()
        .current_dir(&dir)
        .args(["init", "--name", "test"])
        .assert()
        .success();

    let profile_path = dir.path().join("actng.json");
    let mut profile = actng_core::Profile::load(&profile_path).unwrap();
    profile.learn("COOP LAUSANNE", "groceries");
    profile.save(&profile_path).unwrap();

    actng()
        .current_dir(&dir)
        .args(["tags", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("groceries"))
        .stdout(predicate::str::is_match(r"groceries\s+-\s+1\s+1").unwrap());

    actng()
        .current_dir(&dir)
        .args(["tags", "rm", "groceries", "--yes"])
        .assert()
        .success();

    actng()
        .current_dir(&dir)
        .args(["tags", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("groceries").not());
}

#[test]
fn export_writes_quoted_csv_with_all_columns_including_untagged_rows() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture(dir.path());
    actng()
        .current_dir(&dir)
        .args(["init", "--name", "test"])
        .assert()
        .success();

    let profile_path = dir.path().join("actng.json");
    let mut profile = actng_core::Profile::load(&profile_path).unwrap();
    profile.learn("COOP LAUSANNE", "groceries");
    profile.set_category("groceries", "living").unwrap();
    profile.save(&profile_path).unwrap();

    actng()
        .current_dir(&dir)
        .args(["export", "-o", "out.csv", "--summary"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Exported 3 entries"));

    let mut rdr = csv::Reader::from_path(dir.path().join("out.csv")).unwrap();
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

    let records: Vec<csv::StringRecord> = rdr.records().collect::<Result<_, _>>().unwrap();
    assert_eq!(
        records.len(),
        3,
        "every dataset entry exports, tagged or not"
    );

    let coop = records.iter().find(|r| &r[1] == "COOP LAUSANNE").unwrap();
    assert_eq!(&coop[3], "groceries");
    assert_eq!(&coop[4], "living");

    let salary = records.iter().find(|r| &r[1] == "SALARY PAYMENT").unwrap();
    assert_eq!(
        &salary[3], "",
        "untagged entries export with an empty tag, not dropped"
    );
}

#[test]
fn explicit_profile_flag_overrides_env_var() {
    let dir = tempfile::tempdir().unwrap();

    actng()
        .current_dir(&dir)
        .env("ACTNG_PROFILE", "envprofile.json")
        .args([
            "--profile",
            "flagprofile.json",
            "init",
            "--name",
            "flagwins",
        ])
        .assert()
        .success();

    assert!(dir.path().join("flagprofile.json").exists());
    assert!(!dir.path().join("envprofile.json").exists());
}
