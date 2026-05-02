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
use git_forum::internal::evidence;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::policy::{GuardEntry, GuardRule, Policy};
use git_forum::internal::state_change;
use git_forum::internal::thread;
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

// ---- Linked-thread advisory (Track G) ----

fn make_thread_kind(git: &GitOps, kind: ThreadKind, title: &str) -> String {
    create::create_thread(git, kind, title, None, "human/alice", &fixed_clock()).unwrap()
}

fn link_thread(git: &GitOps, from: &str, to: &str, rel: &str) {
    evidence::add_thread_link(git, from, to, rel, "human/alice", &fixed_clock()).unwrap();
}

/// Walk a thread to its terminal `done` state along the shortest valid
/// path. Lifecycles vary in how many intermediate states stand between
/// the initial state and `done`; the state machine guards reject
/// multi-hop calls, so this helper steps through one at a time.
fn drive_to_done(git: &GitOps, policy: &Policy, thread_id: &str) {
    use git_forum::internal::event;
    loop {
        let state = thread::replay_thread(git, thread_id).unwrap();
        if event::normalize_state_name(&state.status) == "done" {
            break;
        }
        let lifecycle = state.lifecycle();
        let path = event::find_path(lifecycle, &state.status, "done")
            .unwrap_or_else(|| panic!("no path to done from {} for {:?}", state.status, lifecycle));
        let next = path.first().expect("path is empty but state != done");
        state_change::change_state(
            git,
            thread_id,
            next,
            &[],
            "human/alice",
            &fixed_clock(),
            policy,
            state_change::StateChangeOptions::default(),
        )
        .unwrap();
    }
}

#[test]
fn verify_surfaces_linked_thread_advisory_when_target_not_done() {
    let (_repo, git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();

    let task = make_thread_kind(&git, ThreadKind::Task, "Implementing task");
    let rfc = make_thread_kind(&git, ThreadKind::Rfc, "Parent RFC");
    link_thread(&git, &task, &rfc, "implements");

    let report = verify::verify_thread(&git, &task, &policy).unwrap();

    // Per SPEC-2.0 §9.4, the verify advisory is informational. The verify
    // result for the named thread is decided by single-thread guards only.
    assert!(
        !report.linked_advisories.is_empty(),
        "expected at least one linked-thread advisory"
    );
    let adv = &report.linked_advisories[0];
    assert_eq!(adv.linked_thread_id, rfc);
    assert!(adv.message.contains("not yet `done`"));
}

#[test]
fn verify_omits_advisory_when_linked_thread_is_done() {
    let (_repo, git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();

    let task = make_thread_kind(&git, ThreadKind::Task, "Implementing task");
    let rfc = make_thread_kind(&git, ThreadKind::Rfc, "Parent RFC");
    link_thread(&git, &task, &rfc, "implements");

    drive_to_done(&git, &policy, &rfc);

    let report = verify::verify_thread(&git, &task, &policy).unwrap();
    assert!(
        report.linked_advisories.is_empty(),
        "advisory should not fire when linked thread is `done`: {:?}",
        report.linked_advisories
    );
}
