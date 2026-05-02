//! Module integration tests for `src/internal/verify.rs`
//! (test-policy.md category 1). DEC/TASK verify cases from the m6
//! split and Track G's linked-thread advisory land here in later
//! commits.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::policy::{GuardEntry, GuardRule, Policy};
use git_forum::internal::state_change;
use git_forum::internal::verify;
use git_forum::internal::write_ops;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

fn make_rfc(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Rfc,
        "Test RFC",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap()
}

fn make_dec(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Dec,
        "Test DEC",
        Some(
            "## Context\nSome context\n## Decision\nUse Redis\n## Rationale\nFast\n## Impact\nNone",
        ),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap()
}

fn make_task(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Task,
        "Test TASK",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap()
}

fn dec_guard_policy() -> Policy {
    make_policy(vec![GuardEntry {
        on: "proposed->accepted".into(),
        requires: vec![GuardRule::NoOpenObjections],
        ..Default::default()
    }])
}

fn task_guard_policy() -> Policy {
    make_policy(vec![GuardEntry {
        on: "reviewing->closed".into(),
        requires: vec![GuardRule::NoOpenActions],
        ..Default::default()
    }])
}

fn make_policy(guards: Vec<GuardEntry>) -> Policy {
    let mut p = Policy {
        guards,
        ..Default::default()
    };
    p.resolve_guard_scopes();
    p
}

fn policy_with_guards() -> Policy {
    make_policy(vec![GuardEntry {
        on: "under-review->accepted".into(),
        requires: vec![GuardRule::NoOpenObjections, GuardRule::OneHumanApproval],
        ..Default::default()
    }])
}

fn empty_policy() -> Policy {
    Policy {
        guards: vec![],
        ..Default::default()
    }
}

fn move_rfc_to_under_review(git: &GitOps, thread_id: &str) {
    state_change::change_state(
        git,
        thread_id,
        "proposed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        git,
        thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
}

#[test]
fn verify_passes_no_guards_configured() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    move_rfc_to_under_review(&git, &thread_id);

    let report = verify::verify_thread(&git, &thread_id, &empty_policy()).unwrap();
    assert!(report.passed());
}

#[test]
fn verify_reports_open_objection_violation() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    move_rfc_to_under_review(&git, &thread_id);

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Not ready.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let report = verify::verify_thread(&git, &thread_id, &policy_with_guards()).unwrap();
    assert!(!report.passed());
    assert!(report
        .violations
        .iter()
        .any(|v| v.rule == "no_open_objections"));
}

#[test]
fn verify_reports_open_action_violation_for_issue_close() {
    let (_repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Implement engine",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Action,
        "Implement parser",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let policy = make_policy(vec![GuardEntry {
        on: "open->closed".into(),
        requires: vec![GuardRule::NoOpenActions],
        ..Default::default()
    }]);

    let report = verify::verify_thread(&git, &thread_id, &policy).unwrap();
    assert!(!report.passed());
    assert!(report
        .violations
        .iter()
        .any(|v| v.rule == "no_open_actions"));
}

// ---- DEC / TASK ----

#[test]
fn verify_dec_proposed_checks_accepted() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    write_ops::say_node(
        &git,
        &id,
        NodeType::Objection,
        "Missing rationale",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let report = verify::verify_thread(&git, &id, &dec_guard_policy()).unwrap();
    assert!(!report.passed());
    assert!(report
        .violations
        .iter()
        .any(|v| v.rule == "no_open_objections"));
}

#[test]
fn verify_task_reviewing_checks_closed() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    for target in &["working", "review"] {
        state_change::change_state(
            &git,
            &id,
            target,
            &[],
            "human/alice",
            &fixed_clock(),
            &empty_policy(),
            state_change::StateChangeOptions::default(),
        )
        .unwrap();
    }
    write_ops::say_node(
        &git,
        &id,
        NodeType::Action,
        "Write tests",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let report = verify::verify_thread(&git, &id, &task_guard_policy()).unwrap();
    assert!(!report.passed());
    assert!(report
        .violations
        .iter()
        .any(|v| v.rule == "no_open_actions"));
}

#[test]
fn verify_task_open_passes_trivially() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let report = verify::verify_thread(&git, &id, &task_guard_policy()).unwrap();
    assert!(report.passed());
}
