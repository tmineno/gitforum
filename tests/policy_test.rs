//! Module integration tests for `src/internal/policy.rs` —
//! file loading, lint diagnostics, and facet-scoped guard resolution
//! (test-policy.md category 1). Tests that exercise the
//! `operation_check.rs` rule tables live in `operation_check_test.rs`.

mod support;

use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::policy::{GuardEntry, GuardRule, Policy};
use git_forum::internal::state_change;
use git_forum::internal::thread;

use support::forum::{fixed_clock, make_policy, setup};

// ---- Loading from file ----

#[test]
fn policy_loads_from_toml_file() {
    let (_repo, _git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap();
    assert!(!policy.guards.is_empty());
    let rfc_state = thread::ThreadState {
        kind: ThreadKind::Rfc,
        ..Default::default()
    };
    let guard = policy.guards_for("under-review->accepted", &rfc_state);
    assert!(!guard.is_empty());
}

#[test]
fn policy_lint_on_default_policy_passes() {
    use git_forum::internal::policy::LintLevel;
    let (_repo, _git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap();
    let diags = git_forum::internal::policy::lint_policy(&policy);
    let warnings: Vec<_> = diags
        .iter()
        .filter(|d| d.level == LintLevel::Warn)
        .collect();
    assert!(warnings.is_empty(), "lint warnings: {warnings:?}");
}

// ---- Kind-scoped guard keys (ISSUE-0097) ----

#[test]
fn scoped_guard_only_fires_for_specified_kind() {
    let (_repo, git, _paths) = setup();

    let issue_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("body"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let task_id = create::create_thread(
        &git,
        ThreadKind::Task,
        "Test task",
        Some("body"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    // Policy: only `bug`-tagged execution threads need commit evidence.
    // SPEC-2.0 §7.1: facet predicate scopes guards by tag, not by kind.
    let policy = make_policy(vec![GuardEntry {
        on: "lifecycle=execution AND tag=bug : open->closed".into(),
        requires: vec![GuardRule::HasCommitEvidence],
        ..Default::default()
    }]);

    let bug_facet = git_forum::internal::write_ops::write_facet_set(
        &git,
        &issue_id,
        None,
        &["bug".to_string()],
        &[],
        "human/alice",
        &fixed_clock(),
    );
    bug_facet.unwrap();

    let result = state_change::change_state(
        &git,
        &issue_id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &policy,
        state_change::StateChangeOptions::default(),
    );
    assert!(
        result.is_err(),
        "tagged-bug issue close should be blocked by tag-scoped guard"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("has_commit_evidence"),
        "error should mention the guard rule"
    );

    let result = state_change::change_state(
        &git,
        &task_id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &policy,
        state_change::StateChangeOptions::default(),
    );
    assert!(
        result.is_ok(),
        "untagged task close should not be blocked by bug-scoped guard"
    );
}

#[test]
fn union_of_scoped_and_unscoped_guards() {
    let (_repo, git, _paths) = setup();

    let issue_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("body"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    // Policy: unscoped open->closed requires no_open_actions,
    // plus issue-scoped requires has_commit_evidence (both apply to issues).
    let policy = make_policy(vec![
        GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        },
        GuardEntry {
            on: "issue:open->closed".into(),
            requires: vec![GuardRule::HasCommitEvidence],
            ..Default::default()
        },
    ]);

    let result = state_change::change_state(
        &git,
        &issue_id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &policy,
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err(), "issue close should be blocked");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("has_commit_evidence"),
        "error should include scoped guard violation"
    );
}
