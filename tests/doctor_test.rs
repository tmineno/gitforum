//! Module integration tests for `src/internal/doctor.rs`
//! (test-policy.md category 1). Track G's "parent done but children
//! open" advisory tests will land here in a later split.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::doctor::{self, CheckLevel};
use git_forum::internal::event::ThreadKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::reindex;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    (repo, git, paths)
}

#[test]
fn doctor_after_init_all_pass() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();

    let report = doctor::run_doctor(&git, &paths).unwrap();
    assert!(
        report.all_passed(),
        "doctor should pass after init: {:?}",
        report
            .checks
            .iter()
            .filter(|c| !c.passed())
            .map(|c| (&c.name, &c.detail))
            .collect::<Vec<_>>()
    );
}

#[test]
fn doctor_uninitialized_reports_failures() {
    let (_repo, git, paths) = setup();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    assert!(!report.all_passed());
}

#[test]
fn doctor_reports_missing_template_files() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    std::fs::remove_file(paths.dot_forum.join("templates/dec.md")).unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let dec_check = report
        .checks
        .iter()
        .find(|c| c.name == "template dec.md")
        .expect("should have dec.md check");
    assert_eq!(dec_check.level, CheckLevel::Fail);
    let issue_check = report
        .checks
        .iter()
        .find(|c| c.name == "template issue.md")
        .expect("should have issue.md check");
    assert_eq!(issue_check.level, CheckLevel::Ok);
}

#[test]
fn doctor_reports_empty_template_file() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    std::fs::write(paths.dot_forum.join("templates/rfc.md"), "").unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let rfc_check = report
        .checks
        .iter()
        .find(|c| c.name == "template rfc.md")
        .expect("should have rfc.md check");
    assert_eq!(rfc_check.level, CheckLevel::Fail);
}

#[test]
fn doctor_warns_on_missing_index() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let idx_check = report
        .checks
        .iter()
        .find(|c| c.name == "index database")
        .expect("should have index database check");
    assert_eq!(idx_check.level, CheckLevel::Warn);
    assert!(report.all_passed());
}

#[test]
fn doctor_passes_index_after_reindex() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let integrity_check = report
        .checks
        .iter()
        .find(|c| c.name == "index integrity")
        .expect("should have integrity check");
    assert_eq!(integrity_check.level, CheckLevel::Ok);
}

#[test]
fn doctor_warns_on_stale_index() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();
    let clock = git_forum::internal::clock::FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    };
    let created_id = git_forum::internal::create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test",
        None,
        "human/alice",
        &clock,
    )
    .unwrap();
    // SPEC-2.0 §6.2: native 2.0 creation produces bare 8-char base36 tokens.
    assert!(
        id_alloc::is_bare_token(&created_id),
        "expected bare token, got: {created_id}"
    );
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let freshness_check = report
        .checks
        .iter()
        .find(|c| c.name == "index freshness")
        .expect("should have freshness check");
    assert_eq!(freshness_check.level, CheckLevel::Warn);
    assert!(report.all_passed());
}
