//! Module integration tests for `src/internal/verify.rs`
//! (test-policy.md category 1). DEC/TASK verify cases from the m6
//! split and Track G's linked-thread advisory land here in later
//! commits.

mod support;

use git_forum::internal::commands::verify;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::policy::{GuardEntry, GuardRule, Policy};
use git_forum::internal::state_change;
use git_forum::internal::write_ops;

use support::forum::{
    dec_guard_policy, drive_to_done, empty_policy, fixed_clock, link_thread, make_dec, make_policy,
    make_rfc, make_task, make_thread, move_rfc_to_under_review, policy_with_under_review_guards,
    setup, task_guard_policy,
};

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

    let report =
        verify::verify_thread(&git, &thread_id, &policy_with_under_review_guards()).unwrap();
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

#[test]
fn verify_surfaces_linked_thread_advisory_when_target_not_done() {
    let (_repo, git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();

    let task = make_thread(&git, ThreadKind::Task, "Implementing task");
    let rfc = make_thread(&git, ThreadKind::Rfc, "Parent RFC");
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

    let task = make_thread(&git, ThreadKind::Task, "Implementing task");
    let rfc = make_thread(&git, ThreadKind::Rfc, "Parent RFC");
    link_thread(&git, &task, &rfc, "implements");

    drive_to_done(&git, &policy, &rfc);

    let report = verify::verify_thread(&git, &task, &policy).unwrap();
    assert!(
        report.linked_advisories.is_empty(),
        "advisory should not fire when linked thread is `done`: {:?}",
        report.linked_advisories
    );
}
