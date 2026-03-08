mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id::SequentialIdGenerator;
use git_forum::internal::init;
use git_forum::internal::policy::{GuardEntry, GuardRule, Policy};
use git_forum::internal::say;
use git_forum::internal::show;
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
    let ids = SequentialIdGenerator::new("e");
    create::create_thread(
        git,
        ThreadKind::Rfc,
        "Test RFC",
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap()
}

fn policy_with_guards() -> Policy {
    use std::collections::HashMap;
    Policy {
        roles: HashMap::new(),
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
    use std::collections::HashMap;
    Policy {
        roles: HashMap::new(),
        guards: vec![],
    }
}

// ---- say / node creation ----

#[test]
fn say_creates_node_in_replay() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "This is needed for compatibility.",
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

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
    let ids = SequentialIdGenerator::new("n");

    say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Benchmarks are missing.",
        "human/bob",
        &fixed_clock(),
        &ids,
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
    let ids = SequentialIdGenerator::new("n");

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Benchmarks are missing.",
        "human/bob",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    say::resolve_node(
        &git,
        &thread_id,
        &node_id,
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.open_objections().is_empty());
    assert!(!state.nodes[0].is_open());
    assert!(state.nodes[0].resolved);
}

#[test]
fn reopen_restores_to_open_objections() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Performance concern.",
        "human/bob",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    say::resolve_node(
        &git,
        &thread_id,
        &node_id,
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();
    say::reopen_node(
        &git,
        &thread_id,
        &node_id,
        "human/bob",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.open_objections().len(), 1);
    assert!(state.nodes[0].is_open());
}

#[test]
fn retract_removes_node_from_open() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Withdrawn concern.",
        "human/bob",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    say::retract_node(
        &git,
        &thread_id,
        &node_id,
        "human/bob",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.open_objections().is_empty());
    assert!(state.nodes[0].retracted);
}

#[test]
fn revise_updates_node_body() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "Initial claim.",
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    say::revise_node(
        &git,
        &thread_id,
        &node_id,
        "Revised claim with more detail.",
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.nodes[0].body, "Revised claim with more detail.");
}

#[test]
fn latest_summary_tracks_most_recent() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    say::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "First summary.",
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    say::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "Second summary.",
        "human/alice",
        &fixed_clock(),
        &ids,
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
    let ids = SequentialIdGenerator::new("n");

    say::say_node(
        &git,
        &thread_id,
        NodeType::Action,
        "Run benchmarks.",
        "human/alice",
        &fixed_clock(),
        &ids,
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
    let ids = SequentialIdGenerator::new("n");

    // Move to under-review (no guards on this transition in empty_policy)
    say::change_state(
        &git,
        &thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &ids,
        &empty_policy(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "under-review");
}

#[test]
fn change_state_invalid_transition_fails() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    let result = say::change_state(
        &git,
        &thread_id,
        "accepted", // draft->accepted is not valid
        &[],
        "human/alice",
        &fixed_clock(),
        &ids,
        &empty_policy(),
    );
    assert!(result.is_err());
}

#[test]
fn change_state_fails_guard_no_open_objections() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    // Move to under-review first
    say::change_state(
        &git,
        &thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &ids,
        &empty_policy(),
    )
    .unwrap();

    // Add an open objection
    say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Missing benchmarks.",
        "human/bob",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    // Policy requires no_open_objections
    use std::collections::HashMap;
    let policy = Policy {
        roles: HashMap::new(),
        guards: vec![GuardEntry {
            on: "under-review->accepted".into(),
            requires: vec![GuardRule::NoOpenObjections],
        }],
    };

    let result = say::change_state(
        &git,
        &thread_id,
        "accepted",
        &["human/alice".to_string()],
        "human/alice",
        &fixed_clock(),
        &ids,
        &policy,
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no_open_objections"));
}

#[test]
fn change_state_passes_all_guards() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    // Move to under-review
    say::change_state(
        &git,
        &thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &ids,
        &empty_policy(),
    )
    .unwrap();

    // Add a summary (satisfies at_least_one_summary)
    say::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "Consensus reached.",
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    // All guards satisfied: no open objections, has summary, has human approval
    say::change_state(
        &git,
        &thread_id,
        "accepted",
        &["human/alice".to_string()],
        "human/alice",
        &fixed_clock(),
        &ids,
        &policy_with_guards(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "accepted");
}

// ---- verify ----

#[test]
fn verify_passes_no_guards_configured() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    // Move to under-review so verify has a forward target
    say::change_state(
        &git,
        &thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &ids,
        &empty_policy(),
    )
    .unwrap();

    let report = verify::verify_thread(&git, &thread_id, &empty_policy()).unwrap();
    assert!(report.passed());
}

#[test]
fn verify_reports_open_objection_violation() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    // Move to under-review
    say::change_state(
        &git,
        &thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &ids,
        &empty_policy(),
    )
    .unwrap();

    // Add open objection
    say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Not ready.",
        "human/bob",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    let report = verify::verify_thread(&git, &thread_id, &policy_with_guards()).unwrap();
    assert!(!report.passed());
    assert!(report
        .violations
        .iter()
        .any(|v| v.rule == "no_open_objections"));
}

// ---- show with nodes ----

#[test]
fn show_includes_open_objections_section() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);
    let ids = SequentialIdGenerator::new("n");

    say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Concern about performance.",
        "human/bob",
        &fixed_clock(),
        &ids,
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
    let ids = SequentialIdGenerator::new("n");

    say::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "This is the consensus.",
        "human/alice",
        &fixed_clock(),
        &ids,
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
    let ids = SequentialIdGenerator::new("n");

    say::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "This is important.",
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state);

    assert!(out.contains("say"));
    assert!(out.contains("claim"));
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
