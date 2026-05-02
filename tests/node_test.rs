//! Module integration tests for `src/internal/node.rs` and the
//! `say_node*` / `revise_node` / `resolve_node` / `reopen_node` /
//! `retract_node` write paths in `src/internal/write_ops.rs`
//! (test-policy.md category 1). Also covers global / scoped node-id
//! resolution exposed by `src/internal/thread.rs`.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::{Clock, FixedClock};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{self, Event, EventType, NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::show;
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

// ---- say / canonicalization ----

#[test]
fn say_node_canonicalizes_legacy_type_and_records_label() {
    // SPEC-2.0 §2.5: every legacy NodeType written via say_node should be
    // stored on the wire as `Comment` with the rhetorical label preserved
    // in the event's legacy_subtype field.
    let cases = [
        (NodeType::Claim, "claim"),
        (NodeType::Question, "question"),
        (NodeType::Evidence, "evidence"),
        (NodeType::Summary, "summary"),
        (NodeType::Risk, "risk"),
        (NodeType::Review, "review"),
        (NodeType::Alternative, "alternative"),
        (NodeType::Assumption, "assumption"),
    ];
    for (input, label) in cases {
        let (_repo, git, _paths) = setup();
        let thread_id = make_rfc(&git);
        write_ops::say_node(
            &git,
            &thread_id,
            input,
            "body",
            "human/alice",
            &fixed_clock(),
            None,
        )
        .unwrap();
        let state = thread::replay_thread(&git, &thread_id).unwrap();
        assert_eq!(
            state.nodes[0].node_type,
            NodeType::Comment,
            "input={input:?}"
        );
        assert_eq!(
            state.nodes[0].legacy_subtype.as_deref(),
            Some(label),
            "input={input:?}"
        );
    }
}

#[test]
fn say_node_canonical_types_pass_through_unchanged() {
    let canonicals = [
        NodeType::Comment,
        NodeType::Objection,
        NodeType::Action,
        NodeType::Approval,
    ];
    for nt in canonicals {
        let (_repo, git, _paths) = setup();
        let thread_id = make_rfc(&git);
        write_ops::say_node(
            &git,
            &thread_id,
            nt,
            "body",
            "human/alice",
            &fixed_clock(),
            None,
        )
        .unwrap();
        let state = thread::replay_thread(&git, &thread_id).unwrap();
        assert_eq!(state.nodes[0].node_type, nt);
        assert_eq!(state.nodes[0].legacy_subtype, None);
    }
}

#[test]
fn say_creates_node_in_replay() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = write_ops::say_node(
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
    // SPEC-2.0 §2.5: the legacy `Claim` input is canonicalized to `Comment` on
    // write; the rhetorical label is preserved on the event (legacy_subtype).
    assert_eq!(state.nodes[0].node_type, NodeType::Comment);
    assert_eq!(state.nodes[0].body, "This is needed for compatibility.");
    assert!(state.nodes[0].is_open());
}

#[test]
fn say_node_with_timestamp_uses_override() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let custom_ts = Utc.with_ymd_and_hms(2020, 3, 10, 8, 0, 0).unwrap();
    write_ops::say_node_with_timestamp(
        &git,
        &id,
        NodeType::Claim,
        "Imported comment",
        "human/bob",
        &fixed_clock(),
        None,
        custom_ts,
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.events[1].created_at, custom_ts);
    assert_eq!(state.nodes[0].created_at, custom_ts);
}

// ---- objection lifecycle ----

#[test]
fn objection_appears_in_open_objections() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    write_ops::say_node(
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

    let node_id = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Benchmarks are missing.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    write_ops::resolve_node(&git, &thread_id, &node_id, "human/alice", &fixed_clock()).unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.open_objections().is_empty());
    assert!(!state.nodes[0].is_open());
    assert!(state.nodes[0].resolved);
}

#[test]
fn reopen_restores_to_open_objections() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Performance concern.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    write_ops::resolve_node(&git, &thread_id, &node_id, "human/alice", &fixed_clock()).unwrap();
    write_ops::reopen_node(&git, &thread_id, &node_id, "human/bob", &fixed_clock()).unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.open_objections().len(), 1);
    assert!(state.nodes[0].is_open());
}

#[test]
fn retract_removes_node_from_open() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Withdrawn concern.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    write_ops::retract_node(&git, &thread_id, &node_id, "human/bob", &fixed_clock()).unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.open_objections().is_empty());
    assert!(state.nodes[0].retracted);
}

// ---- revise / summary / actions ----

#[test]
fn revise_updates_node_body() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "Initial claim.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    write_ops::revise_node(
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

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "First summary.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    write_ops::say_node(
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

    write_ops::say_node(
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

// ---- find_node / resolve_node_id ----

#[test]
fn find_node_returns_current_body_and_history() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Question,
        "What is this?",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    write_ops::revise_node(
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

    let out = show::render_node_show(&lookup, &show::ShowOptions::default());
    assert!(out.contains("What is this object?"));
    assert!(out.contains("What is this?"));
    assert!(out.contains(&node_id[..node_id.len().min(16)]));
    assert!(out.contains("edit"));
    assert!(out.contains("### history"));
    assert!(out.contains("node_id"));
    assert!(out.contains("event_id"));
}

#[test]
fn find_node_accepts_unique_global_prefix() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = write_ops::say_node(
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

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Question,
        "What is this?",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let node_id = write_ops::say_node(
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
    assert!(err.to_string().contains("too short"));
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
        thread_id: first_thread_id.clone(),
        event_type: EventType::Say,
        created_at: fixed_clock().now(),
        actor: "human/alice".into(),
        body: Some("First objection.".into()),
        node_type: Some(NodeType::Objection),
        target_node_id: Some("deadbeef11111111111111111111111111111111".into()),
        ..Event::default()
    };
    let second_event = Event {
        thread_id: second_thread_id.clone(),
        event_type: EventType::Say,
        created_at: fixed_clock().now(),
        actor: "human/bob".into(),
        body: Some("Second objection.".into()),
        node_type: Some(NodeType::Objection),
        target_node_id: Some("deadbeef22222222222222222222222222222222".into()),
        ..Event::default()
    };
    event::write_event(&git, &first_event).unwrap();
    event::write_event(&git, &second_event).unwrap();

    let err = thread::resolve_node_id_global(&git, "deadbeef").unwrap_err();
    assert!(err.to_string().contains("ambiguous"));

    let resolved = thread::resolve_node_id_in_thread(&git, &first_thread_id, "deadbeef").unwrap();
    assert_eq!(resolved, "deadbeef11111111111111111111111111111111");
}

// ---- DEC / TASK rhetorical types ----

#[test]
fn node_add_alternative() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let node_id = write_ops::say_node(
        &git,
        &id,
        NodeType::Alternative,
        "Use Memcached instead",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.nodes.len(), 1);
    assert_eq!(state.nodes[0].node_id, node_id);
    // SPEC-2.0 §2.5: `Alternative` is a legacy rhetorical type that
    // canonicalizes to `Comment` on write; the label is preserved.
    assert_eq!(state.nodes[0].node_type, NodeType::Comment);
    assert_eq!(
        state.nodes[0].legacy_subtype.as_deref(),
        Some("alternative")
    );
    assert_eq!(state.nodes[0].body, "Use Memcached instead");
}

#[test]
fn node_add_assumption() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let node_id = write_ops::say_node(
        &git,
        &id,
        NodeType::Assumption,
        "Redis cluster is available in prod",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.nodes.len(), 1);
    assert_eq!(state.nodes[0].node_id, node_id);
    // SPEC-2.0 §2.5: `Assumption` canonicalizes to `Comment` on write.
    assert_eq!(state.nodes[0].node_type, NodeType::Comment);
    assert_eq!(state.nodes[0].legacy_subtype.as_deref(), Some("assumption"));
    assert_eq!(state.nodes[0].body, "Redis cluster is available in prod");
}
