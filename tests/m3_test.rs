mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::{Clock, FixedClock};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{self, Event, EventType, NodeType, ThreadKind};
use git_forum::internal::evidence::EvidenceKind;
use git_forum::internal::evidence_ops;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::policy::{GuardEntry, GuardRule, Policy};
use git_forum::internal::say;
use git_forum::internal::show;
use git_forum::internal::state_change;
use git_forum::internal::thread;
use git_forum::internal::verify;

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

fn policy_with_guards() -> Policy {
    Policy {
        guards: vec![GuardEntry {
            on: "under-review->accepted".into(),
            requires: vec![
                GuardRule::NoOpenObjections,
                GuardRule::AtLeastOneSummary,
                GuardRule::OneHumanApproval,
            ],
        }],
    }
}

fn empty_policy() -> Policy {
    Policy { guards: vec![] }
}

// ---- say / node creation ----

#[test]
fn say_creates_node_in_replay() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "This is needed for compatibility.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let tip = git
        .resolve_ref(&format!("refs/forum/threads/{thread_id}"))
        .unwrap()
        .unwrap();
    assert_eq!(tip, node_id);

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.nodes.len(), 1);
    assert_eq!(state.nodes[0].node_id, node_id);
    assert_eq!(state.nodes[0].node_type, NodeType::Claim);
    assert_eq!(state.nodes[0].body, "This is needed for compatibility.");
    assert!(state.nodes[0].is_open());
}

#[test]
fn objection_appears_in_open_objections() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Benchmarks are missing.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let open = state.open_objections();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].body, "Benchmarks are missing.");
}

#[test]
fn resolve_removes_from_open_objections() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Benchmarks are missing.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    say::resolve_node(&git, &thread_id, &node_id, "human/alice", &fixed_clock()).unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.open_objections().is_empty());
    assert!(!state.nodes[0].is_open());
    assert!(state.nodes[0].resolved);
}

#[test]
fn reopen_restores_to_open_objections() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Performance concern.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    say::resolve_node(&git, &thread_id, &node_id, "human/alice", &fixed_clock()).unwrap();
    say::reopen_node(&git, &thread_id, &node_id, "human/bob", &fixed_clock()).unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.open_objections().len(), 1);
    assert!(state.nodes[0].is_open());
}

#[test]
fn retract_removes_node_from_open() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Withdrawn concern.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    say::retract_node(&git, &thread_id, &node_id, "human/bob", &fixed_clock()).unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.open_objections().is_empty());
    assert!(state.nodes[0].retracted);
}

#[test]
fn revise_updates_node_body() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "Initial claim.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    say::revise_node(
        &git,
        &thread_id,
        &node_id,
        "Revised claim with more detail.",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.nodes[0].body, "Revised claim with more detail.");
}

#[test]
fn latest_summary_tracks_most_recent() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    say::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "First summary.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    say::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "Second summary.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let s = state.latest_summary().unwrap();
    assert_eq!(s.body, "Second summary.");
}

#[test]
fn open_actions_tracks_unresolved_actions() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    say::say_node(
        &git,
        &thread_id,
        NodeType::Action,
        "Run benchmarks.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.open_actions().len(), 1);
}

// ---- state transitions ----

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
    assert_eq!(state.status, "under-review");
}

#[test]
fn change_state_invalid_transition_fails() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let result = state_change::change_state(
        &git,
        &thread_id,
        "accepted", // draft->accepted is not valid
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("valid transitions from 'draft': [proposed, rejected]"));
}

#[test]
fn change_state_fails_guard_no_open_objections() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    move_rfc_to_under_review(&git, &thread_id);

    // Add an open objection
    say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Missing benchmarks.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    // Policy requires no_open_objections

    let policy = Policy {
        guards: vec![GuardEntry {
            on: "under-review->accepted".into(),
            requires: vec![GuardRule::NoOpenObjections],
        }],
    };

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

    // Add a summary (satisfies at_least_one_summary)
    say::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "Consensus reached.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    // All guards satisfied: no open objections, has summary, has human approval
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
    assert_eq!(state.status, "accepted");
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

    say::say_node(
        &git,
        &thread_id,
        NodeType::Action,
        "Implement parser",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let policy = Policy {
        guards: vec![GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
        }],
    };

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

    say::say_node(
        &git,
        &thread_id,
        NodeType::Action,
        "Implement parser",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let policy = Policy {
        guards: vec![GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
        }],
    };

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
        },
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "closed");
    assert_eq!(state.open_actions().len(), 0);
}

// ---- verify ----

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

    // Add open objection
    say::say_node(
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

    say::say_node(
        &git,
        &thread_id,
        NodeType::Action,
        "Implement parser",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let policy = Policy {
        guards: vec![GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
        }],
    };

    let report = verify::verify_thread(&git, &thread_id, &policy).unwrap();
    assert!(!report.passed());
    assert!(report
        .violations
        .iter()
        .any(|v| v.rule == "no_open_actions"));
}

// ---- show with nodes ----

#[test]
fn show_includes_open_objections_section() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Concern about performance.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state);

    assert!(out.contains("open objections: 1"));
    assert!(out.contains("Concern about performance."));
}

#[test]
fn show_includes_latest_summary_section() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    say::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "This is the consensus.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state);

    assert!(out.contains("latest summary:"));
    assert!(out.contains("This is the consensus."));
}

#[test]
fn show_no_extra_sections_when_no_nodes() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state);

    assert!(!out.contains("open objections:"));
    assert!(!out.contains("open actions:"));
    assert!(!out.contains("latest summary:"));
}

#[test]
fn show_timeline_includes_say_events() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "This is important.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state);

    assert!(out.contains(&node_id[..node_id.len().min(16)]));
    assert!(out.contains("claim"));
    assert!(out.contains("This is important."));
}

#[test]
fn find_node_returns_current_body_and_history() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Question,
        "What is this?",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    say::revise_node(
        &git,
        &thread_id,
        &node_id,
        "What is this object?",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let lookup = thread::find_node(&git, &node_id).unwrap();
    assert_eq!(lookup.thread_id, thread_id);
    assert_eq!(lookup.node.node_id, node_id);
    assert_eq!(lookup.node.body, "What is this object?");
    assert_eq!(lookup.events.len(), 2);

    let out = show::render_node_show(&lookup);
    assert!(out.contains("What is this object?"));
    assert!(out.contains("What is this?"));
    assert!(out.contains(&node_id[..node_id.len().min(16)]));
    assert!(out.contains("edit"));
    assert!(out.contains("history:"));
    assert!(out.contains("node_id"));
    assert!(out.contains("event_id"));
}

#[test]
fn find_node_accepts_unique_global_prefix() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Question,
        "What is this?",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let prefix = &node_id[..thread::MIN_NODE_ID_PREFIX_LEN];
    let lookup = thread::find_node(&git, prefix).unwrap();
    assert_eq!(lookup.node.node_id, node_id);
}

#[test]
fn resolve_node_id_rejects_short_prefix() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    say::say_node(
        &git,
        &thread_id,
        NodeType::Question,
        "What is this?",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Question,
        "What is this?",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let short_prefix = &node_id[..thread::MIN_NODE_ID_PREFIX_LEN - 1];
    let err = thread::resolve_node_id_global(&git, short_prefix).unwrap_err();
    assert!(err.to_string().contains("at least 8 characters"));
}

#[test]
fn resolve_node_id_in_thread_scopes_prefix_lookup() {
    let (_repo, git, _paths) = setup();
    let first_thread_id = make_rfc(&git);
    let second_thread_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Second RFC",
        None,
        "human/bob",
        &fixed_clock(),
    )
    .unwrap();

    let first_event = Event {
        event_id: String::new(),
        thread_id: first_thread_id.clone(),
        event_type: EventType::Say,
        created_at: fixed_clock().now(),
        actor: "human/alice".into(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: Some("First objection.".into()),
        node_type: Some(NodeType::Objection),
        target_node_id: Some("deadbeef11111111111111111111111111111111".into()),
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        branch: None,
        incorporated_node_ids: vec![],
        reply_to: None,
    };
    let second_event = Event {
        event_id: String::new(),
        thread_id: second_thread_id.clone(),
        event_type: EventType::Say,
        created_at: fixed_clock().now(),
        actor: "human/bob".into(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: Some("Second objection.".into()),
        node_type: Some(NodeType::Objection),
        target_node_id: Some("deadbeef22222222222222222222222222222222".into()),
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        branch: None,
        incorporated_node_ids: vec![],
        reply_to: None,
    };
    event::write_event(&git, &first_event).unwrap();
    event::write_event(&git, &second_event).unwrap();

    let err = thread::resolve_node_id_global(&git, "deadbeef").unwrap_err();
    assert!(err.to_string().contains("ambiguous"));

    let resolved = thread::resolve_node_id_in_thread(&git, &first_thread_id, "deadbeef").unwrap();
    assert_eq!(resolved, "deadbeef11111111111111111111111111111111");
}

// ---- issue rejected state ----

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

    let policy = Policy {
        guards: vec![GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::HasCommitEvidence],
        }],
    };

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

    // Create a real commit in the test repo to use as evidence
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

    // Add commit evidence
    evidence_ops::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::Commit,
        "HEAD",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let policy = Policy {
        guards: vec![GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::HasCommitEvidence],
        }],
    };

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
    assert_eq!(state.status, "closed");
}

// ---- policy loading from file ----

#[test]
fn policy_loads_from_toml_file() {
    let (_repo, _git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap();
    // Default policy has one guard entry for under-review->accepted
    assert!(!policy.guards.is_empty());
    let guard = policy.guards_for("under-review->accepted");
    assert!(!guard.is_empty());
}

#[test]
fn policy_lint_on_default_policy_passes() {
    let (_repo, _git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap();
    let diags = git_forum::internal::policy::lint_policy(&policy);
    assert!(diags.is_empty(), "lint diags: {diags:?}");
}

// ---- RFC deprecated state ----

#[test]
fn rfc_accepted_then_deprecated() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    // Move through draft -> proposed -> under-review -> accepted -> deprecated
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

    // draft -> rejected -> deprecated
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

    // Create source RFC, accept it
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

    // Replay source to get title/body
    let source_state = thread::replay_thread(&git, &source_id).unwrap();

    // Create new RFC "from" source (simulating --from-thread)
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

    // Add links: new supersedes source, source superseded-by new
    evidence_ops::add_thread_link(
        &git,
        &new_id,
        &source_id,
        "supersedes",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    evidence_ops::add_thread_link(
        &git,
        &source_id,
        &new_id,
        "superseded-by",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    // Auto-deprecate source
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

    // Verify new RFC
    let new_state = thread::replay_thread(&git, &new_id).unwrap();
    assert_eq!(new_state.title, "v2: Original design");
    assert_eq!(new_state.body.as_deref(), Some("Body of original RFC"));
    assert_eq!(new_state.links.len(), 1);
    assert_eq!(new_state.links[0].target_thread_id, source_id);
    assert_eq!(new_state.links[0].rel, "supersedes");

    // Verify source RFC is deprecated with backlink
    let source_after = thread::replay_thread(&git, &source_id).unwrap();
    assert_eq!(source_after.status, "deprecated");
    assert_eq!(source_after.links.len(), 1);
    assert_eq!(source_after.links[0].target_thread_id, new_id);
    assert_eq!(source_after.links[0].rel, "superseded-by");
}
