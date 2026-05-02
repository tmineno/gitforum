//! Module integration tests for `src/internal/state_change.rs`
//! (test-policy.md category 1). Covers transition validation,
//! guard enforcement, fast-track walking, approvals, and the RFC
//! deprecated/superseded path. DEC and TASK lifecycle coverage from
//! the m6 split lands here as well.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::{Clock, FixedClock};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{self, Event, EventType, NodeType, ThreadKind};
use git_forum::internal::evidence;
use git_forum::internal::evidence::EvidenceKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::policy::{GuardEntry, GuardRule, Policy};
use git_forum::internal::state_change;
use git_forum::internal::thread;
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
    // ADR-006: `AtLeastOneSummary` removed in 2.0; remaining guards
    // exercise the same review→done transition.
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

// ---- Transitions ----

#[test]
fn change_state_valid_transition_no_guards() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    state_change::change_state(
        &git,
        &thread_id,
        "proposed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    state_change::change_state(
        &git,
        &thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "review");
}

#[test]
fn change_state_invalid_transition_fails() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let result = state_change::change_state(
        &git,
        &thread_id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("valid transitions from 'draft': [open, withdrawn]"));
}

#[test]
fn change_state_fails_guard_no_open_objections() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    move_rfc_to_under_review(&git, &thread_id);

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Missing benchmarks.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let policy = make_policy(vec![GuardEntry {
        on: "under-review->accepted".into(),
        requires: vec![GuardRule::NoOpenObjections],
        ..Default::default()
    }]);

    let result = state_change::change_state(
        &git,
        &thread_id,
        "accepted",
        &["human/alice".to_string()],
        "human/alice",
        &fixed_clock(),
        &policy,
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no_open_objections"));
}

#[test]
fn change_state_passes_all_guards() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    move_rfc_to_under_review(&git, &thread_id);

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "Consensus reached.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    state_change::change_state(
        &git,
        &thread_id,
        "accepted",
        &["human/alice".to_string()],
        "human/alice",
        &fixed_clock(),
        &policy_with_guards(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "done");
}

#[test]
fn change_state_issue_close_fails_guard_no_open_actions() {
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

    let err = state_change::change_state(
        &git,
        &thread_id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &policy,
        state_change::StateChangeOptions::default(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("no_open_actions"));
}

#[test]
fn change_state_issue_close_can_resolve_open_actions() {
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

    state_change::change_state(
        &git,
        &thread_id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &policy,
        state_change::StateChangeOptions {
            resolve_open_actions: true,
            ..Default::default()
        },
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "done");
    assert_eq!(state.open_actions().len(), 0);
}

// ---- Issue rejected / commit-evidence ----

#[test]
fn change_state_issue_rejected_succeeds() {
    let (_repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Invalid issue",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    state_change::change_state(
        &git,
        &thread_id,
        "rejected",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "rejected");
}

#[test]
fn change_state_issue_close_fails_guard_has_commit_evidence() {
    let (_repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Needs commit evidence",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let policy = make_policy(vec![GuardEntry {
        on: "open->closed".into(),
        requires: vec![GuardRule::HasCommitEvidence],
        ..Default::default()
    }]);

    let err = state_change::change_state(
        &git,
        &thread_id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &policy,
        state_change::StateChangeOptions::default(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("has_commit_evidence"));
}

#[test]
fn change_state_issue_close_passes_with_commit_evidence() {
    let (repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Has commit evidence",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    std::fs::write(repo.path().join("test.txt"), "hello").unwrap();
    let add_out = std::process::Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(add_out.status.success());
    let commit_out = std::process::Command::new("git")
        .args(["commit", "-m", "test commit"])
        .current_dir(repo.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(commit_out.status.success());

    evidence::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::Commit,
        "HEAD",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let policy = make_policy(vec![GuardEntry {
        on: "open->closed".into(),
        requires: vec![GuardRule::HasCommitEvidence],
        ..Default::default()
    }]);

    state_change::change_state(
        &git,
        &thread_id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &policy,
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "done");
}

// ---- RFC deprecated / superseded ----

#[test]
fn rfc_accepted_then_deprecated() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    move_rfc_to_under_review(&git, &thread_id);
    state_change::change_state(
        &git,
        &thread_id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &thread_id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "deprecated");
}

#[test]
fn rfc_rejected_then_deprecated() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    // draft → proposed (= open) → rejected → deprecated. The unified §3.1
    // graph has no draft→rejected edge, so the rejection has to flow
    // through `open` first.
    state_change::change_state(
        &git,
        &thread_id,
        "proposed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &thread_id,
        "rejected",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &thread_id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "deprecated");
}

#[test]
fn from_thread_creates_new_rfc_with_links_and_deprecates_source() {
    let (_repo, git, paths) = setup();

    let source_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Original design",
        Some("Body of original RFC"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    move_rfc_to_under_review(&git, &source_id);
    state_change::change_state(
        &git,
        &source_id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let source_state = thread::replay_thread(&git, &source_id).unwrap();

    let new_title = format!("v2: {}", source_state.title);
    let new_body = source_state.body.clone();
    let new_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        &new_title,
        new_body.as_deref(),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    evidence::add_thread_link(
        &git,
        &new_id,
        &source_id,
        "supersedes",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    evidence::add_thread_link(
        &git,
        &source_id,
        &new_id,
        "superseded-by",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap();
    state_change::change_state(
        &git,
        &source_id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &policy,
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let new_state = thread::replay_thread(&git, &new_id).unwrap();
    assert_eq!(new_state.title, "v2: Original design");
    assert_eq!(new_state.body.as_deref(), Some("Body of original RFC"));
    assert_eq!(new_state.links.len(), 1);
    assert_eq!(new_state.links[0].target_thread_id, source_id);
    assert_eq!(new_state.links[0].rel, "supersedes");

    let source_after = thread::replay_thread(&git, &source_id).unwrap();
    assert_eq!(source_after.status, "deprecated");
    assert_eq!(source_after.links.len(), 1);
    assert_eq!(source_after.links[0].target_thread_id, new_id);
    assert_eq!(source_after.links[0].rel, "superseded-by");
}

// ---- fast_track ----

#[test]
fn fast_track_rfc_draft_to_accepted() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "Consensus reached.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let walked = state_change::fast_track_state(
        &git,
        &thread_id,
        "accepted",
        &["human/alice".to_string()],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    // Unified §3.1: draft→open→done is the shortest path (no review hop
    // unless the policy makes review→done the only guarded edge).
    assert_eq!(walked, vec!["open", "done"]);

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "done");
}

#[test]
fn fast_track_emits_separate_events_per_step() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    state_change::fast_track_state(
        &git,
        &thread_id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    // Create + 2 state events (open, done) = 3 events total under §3.1.
    assert_eq!(state.events.len(), 3);
}

#[test]
fn fast_track_stops_on_guard_failure() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    // Construct a policy that requires no_open_actions on the proposal
    // lifecycle's open→done edge specifically (the default policy
    // post-@ltojzq9l only enforces no_open_actions on execution
    // lifecycle, so we need an explicit guard here to test that
    // fast_track stops mid-walk on a guard failure).
    let mut custom_policy: Policy = toml::from_str(
        r#"
[[guards]]
on = "lifecycle=proposal : open->done"
requires = ["no_open_actions"]
"#,
    )
    .unwrap();
    let _ = custom_policy.resolve_guard_scopes();

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Action,
        "Pending follow-up.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let result = state_change::fast_track_state(
        &git,
        &thread_id,
        "accepted",
        &["human/alice".to_string()],
        "human/alice",
        &fixed_clock(),
        &custom_policy,
        state_change::StateChangeOptions::default(),
    );

    assert!(result.is_err());

    // Walk made it to `open` (first step) before the guard on the
    // open→done edge stopped the second step.
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "open");
}

#[test]
fn fast_track_no_path_returns_error() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    state_change::fast_track_state(
        &git,
        &thread_id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let result = state_change::fast_track_state(
        &git,
        &thread_id,
        "draft",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    );

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no path"), "unexpected error: {err}");
}

#[test]
fn fast_track_already_at_target_is_noop() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let walked = state_change::fast_track_state(
        &git,
        &thread_id,
        "draft",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    assert!(walked.is_empty());
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "draft");
}

#[test]
fn fast_track_sign_and_comment_only_on_final_step() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    state_change::fast_track_state(
        &git,
        &thread_id,
        "accepted",
        &["human/alice".to_string()],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions {
            comment: Some("Done!".to_string()),
            ..Default::default()
        },
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "done");
    // SPEC-2.0 §2.8: 2.0 emits approvals as `approval`-typed Say nodes
    // before the final State event, not as fields on the State event.
    let approval_nodes: Vec<_> = state
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Approval)
        .collect();
    assert_eq!(approval_nodes.len(), 1);
    assert_eq!(approval_nodes[0].actor, "human/alice");
    let state_events: Vec<_> = state
        .events
        .iter()
        .filter(|e| e.event_type == EventType::State)
        .collect();
    // Unified §3.1: draft→done is two state events (open, done), not three.
    assert_eq!(state_events.len(), 2);
    for ev in &state_events {
        assert!(ev.approvals.is_empty());
    }
}

// ---- Approvals ----

#[test]
fn change_state_emits_approval_node_per_actor() {
    // SPEC-2.0 §2.8: state transitions emit one Approval-typed Say node per
    // approver. The State event carries no approvals field.
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    state_change::change_state(
        &git,
        &thread_id,
        "proposed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &thread_id,
        "accepted",
        &["human/alice".to_string(), "ai/reviewer".to_string()],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let approvals: Vec<_> = state
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Approval)
        .collect();
    assert_eq!(approvals.len(), 2);
    let actors: Vec<&str> = approvals.iter().map(|n| n.actor.as_str()).collect();
    assert!(actors.contains(&"human/alice"));
    assert!(actors.contains(&"ai/reviewer"));
}

#[test]
fn change_state_dedupes_repeated_approvers() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    state_change::change_state(
        &git,
        &thread_id,
        "proposed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &thread_id,
        "accepted",
        &["human/alice".to_string(), "human/alice".to_string()],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let approvals: Vec<_> = state
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Approval)
        .collect();
    assert_eq!(approvals.len(), 1);
}

#[test]
fn legacy_state_approvals_replay_into_nodes() {
    // 1.x State events stored approvals on the event itself. Replay must
    // synthesize equivalent Approval nodes so policy guards see them
    // (SPEC-2.0 §2.8 / §10.1).
    use git_forum::internal::event::{Approval, ApprovalMechanism};
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let now = fixed_clock().now();
    let legacy_state_event = Event {
        thread_id: thread_id.clone(),
        event_type: EventType::State,
        actor: "human/alice".into(),
        created_at: now,
        new_state: Some("proposed".into()),
        approvals: vec![Approval {
            actor_id: "human/alice".into(),
            approved_at: now,
            mechanism: ApprovalMechanism::Recorded,
            key_id: None,
            proof_ref: None,
        }],
        ..Event::default()
    };
    event::write_event(&git, &legacy_state_event).unwrap();
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let approvals: Vec<_> = state
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Approval)
        .collect();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].actor, "human/alice");
}

#[test]
fn state_change_with_comment_attaches_body_no_summary_node() {
    let (_repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Issue with comment",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    state_change::change_state(
        &git,
        &thread_id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions {
            comment: Some("closing because resolved".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "done");

    assert_eq!(state.events.len(), 2);
    assert_eq!(state.events[0].event_type, EventType::Create);
    assert_eq!(state.events[1].event_type, EventType::State);
    assert_eq!(
        state.events[1].body.as_deref(),
        Some("closing because resolved")
    );

    assert!(state.nodes.is_empty());

    let out = git_forum::internal::show::render_show(
        &state,
        &git_forum::internal::show::ShowOptions::default(),
    );
    assert!(out.contains("done — closing because resolved"));
}

// ---- DEC lifecycle ----

#[test]
fn dec_create_sets_proposed_state() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "open");
    assert_eq!(state.kind, ThreadKind::Dec);
    // SPEC-2.0 §6.2: kind is on the Create event, not the ID.
    assert!(git_forum::internal::id_alloc::is_bare_token(&id));
}

#[test]
fn dec_proposed_to_accepted() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    state_change::change_state(
        &git,
        &id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "done");
}

#[test]
fn dec_proposed_to_rejected() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    state_change::change_state(
        &git,
        &id,
        "rejected",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "rejected");
}

#[test]
fn dec_proposed_to_deprecated() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    // Unified §3.1: open (was proposed) cannot directly deprecate; the only
    // edges into `deprecated` are from `done` and `rejected`. Walk via
    // accepted (= done).
    state_change::change_state(
        &git,
        &id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "deprecated");
}

#[test]
fn dec_accepted_to_deprecated() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    state_change::change_state(
        &git,
        &id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "deprecated");
}

#[test]
fn dec_rejected_to_deprecated() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    state_change::change_state(
        &git,
        &id,
        "rejected",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "deprecated");
}

#[test]
fn dec_proposed_to_pending_is_invalid() {
    // SPEC-2.0 §3.1.1: record lifecycle excludes `working` (and 1.x
    // `pending` normalizes to working). DEC threads cannot enter the
    // working state.
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let result = state_change::change_state(
        &git,
        &id,
        "pending",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    );
    let err = result.unwrap_err();
    // SPEC-2.0 §13: destination state not in record lifecycle's allowed
    // set is reported as LifecycleStateMismatch.
    assert!(
        matches!(
            err,
            git_forum::internal::error::ForumError::LifecycleStateMismatch(_),
        ),
        "expected LifecycleStateMismatch, got {err:?}",
    );
}

#[test]
fn dec_proposed_to_accepted_blocked_by_objection() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    write_ops::say_node(
        &git,
        &id,
        NodeType::Objection,
        "Missing benchmarks",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let result = state_change::change_state(
        &git,
        &id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &dec_guard_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no_open_objections"));
}

// ---- TASK lifecycle ----

#[test]
fn task_create_sets_open_state() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "open");
    assert_eq!(state.kind, ThreadKind::Task);
    assert!(git_forum::internal::id_alloc::is_bare_token(&id));
}

#[test]
fn task_full_lifecycle() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    // Unified §3.1: task lifecycle (execution) folds 1.x designing /
    // implementing into a single `working` state, so the full path is
    // open → working → review → done. (1.x `closed` normalizes to `done`.)
    for target in &["working", "review", "closed"] {
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
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "done");
}

#[test]
fn task_fast_track_open_to_closed() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    state_change::change_state(
        &git,
        &id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "done");
}

// 1.x had separate `designing` / `implementing` states inside Task; both
// fold to `working` in the unified §3.1 graph, so the back-transition
// from implementing to designing is no longer expressible. The remaining
// back-transition coverage runs review → working below.

#[test]
fn task_back_transition_review_to_working() {
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
    state_change::change_state(
        &git,
        &id,
        "working",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "working");
}

#[test]
fn task_invalid_open_to_deprecated() {
    // Unified §3.1 has no open→deprecated edge (deprecated is reachable
    // only from done / rejected).
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let result = state_change::change_state(
        &git,
        &id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
}

#[test]
fn task_invalid_review_to_draft() {
    // Execution lifecycle excludes `draft`; the equivalent invalid edge
    // for execution is review→draft.
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
    let result = state_change::change_state(
        &git,
        &id,
        "draft",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
}

#[test]
fn task_reopen_from_closed() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    state_change::change_state(
        &git,
        &id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "open",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "open");
}

#[test]
fn task_reopen_from_rejected() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    state_change::change_state(
        &git,
        &id,
        "rejected",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "open",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "open");
}

#[test]
fn task_reviewing_to_closed_blocked_by_actions() {
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
        "Add tests",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let result = state_change::change_state(
        &git,
        &id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &task_guard_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no_open_actions"));
}
