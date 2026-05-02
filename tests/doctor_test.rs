//! Module integration tests for `src/internal/doctor.rs`
//! (test-policy.md category 1). Track G's "parent done but children
//! open" advisory tests will land here in a later split.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::doctor::{self, CheckLevel};
use git_forum::internal::event::ThreadKind;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::policy::Policy;
use git_forum::internal::reindex;

use support::forum::{drive_to_done, link_thread, make_thread, setup_no_init as setup};

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

// ---- Linked-thread advisory (Track G) ----

#[test]
fn doctor_surfaces_open_implementing_children_under_done_parent() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();

    let parent = make_thread(&git, ThreadKind::Rfc, "Parent RFC");
    let child_open = make_thread(&git, ThreadKind::Task, "Still working");
    let child_done = make_thread(&git, ThreadKind::Task, "Already done");
    link_thread(&git, &child_open, &parent, "implements");
    link_thread(&git, &child_done, &parent, "implements");

    drive_to_done(&git, &policy, &parent);
    drive_to_done(&git, &policy, &child_done);

    let report = doctor::run_doctor(&git, &paths).unwrap();

    let advisory_match = report
        .advisories
        .iter()
        .find(|a| a.contains(&parent))
        .unwrap_or_else(|| {
            panic!(
                "expected advisory for {parent}, got: {:?}",
                report.advisories
            )
        });
    assert!(
        advisory_match.contains(&child_open),
        "expected open child id in advisory: {advisory_match}"
    );
    assert!(
        !advisory_match.contains(&child_done),
        "done child should not appear in advisory: {advisory_match}"
    );
    assert!(
        advisory_match.contains("1 implementing child still open"),
        "advisory phrasing changed: {advisory_match}"
    );

    // Advisory MUST NOT affect the doctor's pass/fail decision.
    assert!(
        report.all_passed(),
        "advisories should not flip doctor pass/fail"
    );
}

#[test]
fn doctor_quiet_when_no_done_parents_have_open_implementers() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    make_thread(&git, ThreadKind::Rfc, "Lonely RFC");
    let report = doctor::run_doctor(&git, &paths).unwrap();
    assert!(
        report.advisories.is_empty(),
        "no advisories expected: {:?}",
        report.advisories
    );
}
