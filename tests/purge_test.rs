mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::purge;
use git_forum::internal::thread;
use git_forum::internal::write_ops;

fn setup() -> (support::repo::TestRepo, GitOps) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    (repo, git)
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

fn make_thread_with_node(git: &GitOps) -> (String, String) {
    let thread_id = create::create_thread(
        git,
        ThreadKind::Issue,
        "Test Issue",
        Some("Issue body"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let node_sha = write_ops::say_node(
        git,
        &thread_id,
        NodeType::Claim,
        "Secret claim text",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    (thread_id, node_sha)
}

// ---- purge_event tests ----

#[test]
fn purge_event_replaces_body_with_purged() {
    let (_repo, git) = setup();
    let (thread_id, node_sha) = make_thread_with_node(&git);

    let report = purge::purge_event(&git, &thread_id, &node_sha).unwrap();
    assert_eq!(report.events_purged, 1);
    assert!(report.commits_rewritten >= 1);

    // Verify the body was purged in the replayed state
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let node = state.nodes.iter().find(|n| n.body == "[purged]");
    assert!(node.is_some(), "node body should be [purged]");
}

#[test]
fn purge_event_preserves_other_events() {
    let (_repo, git) = setup();
    let (thread_id, node_sha) = make_thread_with_node(&git);

    // Add another node that should NOT be purged
    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "Public claim text",
        "human/charlie",
        &fixed_clock(),
        None,
    )
    .unwrap();

    purge::purge_event(&git, &thread_id, &node_sha).unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    // First node (by bob) should be purged
    assert_eq!(state.nodes[0].body, "[purged]");
    // Second node (by charlie) should be intact
    assert_eq!(state.nodes[1].body, "Public claim text");
}

#[test]
fn purge_event_thread_still_replays() {
    let (_repo, git) = setup();
    let (thread_id, node_sha) = make_thread_with_node(&git);

    purge::purge_event(&git, &thread_id, &node_sha).unwrap();

    // Thread should still replay successfully
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.id, thread_id);
    assert_eq!(state.title, "Test Issue");
    assert_eq!(state.status, "open");
}

#[test]
fn purge_event_not_found_returns_error() {
    let (_repo, git) = setup();
    let (thread_id, _node_sha) = make_thread_with_node(&git);

    let result = purge::purge_event(&git, &thread_id, "0000000000000000");
    assert!(result.is_err());
}

// ---- purge_actor tests ----

#[test]
fn purge_actor_replaces_all_matching_events() {
    let (_repo, git) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test Issue",
        Some("Issue body"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    // Two nodes by bob
    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "Bob claim 1",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "Bob claim 2",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    // One node by charlie (should be preserved)
    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "Charlie claim",
        "human/charlie",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let report = purge::purge_actor(&git, "human/bob").unwrap();
    assert_eq!(report.events_purged, 2);
    assert_eq!(report.threads_affected.len(), 1);

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    // Bob's nodes should be purged
    assert_eq!(state.nodes[0].body, "[purged]");
    assert_eq!(state.nodes[0].actor, "[purged]");
    assert_eq!(state.nodes[1].body, "[purged]");
    assert_eq!(state.nodes[1].actor, "[purged]");
    // Charlie's node intact
    assert_eq!(state.nodes[2].body, "Charlie claim");
    assert_eq!(state.nodes[2].actor, "human/charlie");
}

#[test]
fn purge_actor_across_multiple_threads() {
    let (_repo, git) = setup();
    // Thread 1
    let tid1 = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Issue 1",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    write_ops::say_node(
        &git,
        &tid1,
        NodeType::Claim,
        "Bob in thread 1",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    // Thread 2
    let tid2 = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Issue 2",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    write_ops::say_node(
        &git,
        &tid2,
        NodeType::Claim,
        "Bob in thread 2",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let report = purge::purge_actor(&git, "human/bob").unwrap();
    assert_eq!(report.events_purged, 2);
    assert_eq!(report.threads_affected.len(), 2);
}

#[test]
fn purge_actor_no_match_returns_zero() {
    let (_repo, git) = setup();
    let _tid = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Issue",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let report = purge::purge_actor(&git, "human/nonexistent").unwrap();
    assert_eq!(report.events_purged, 0);
    assert!(report.threads_affected.is_empty());
}

// ---- purge by node ID (resolve node → event SHA) ----

#[test]
fn purge_by_node_id_resolves_to_event_and_purges() {
    let (_repo, git) = setup();
    let (thread_id, node_sha) = make_thread_with_node(&git);

    // node_sha is both the event_id and the node_id for a Say event
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let node = state
        .nodes
        .iter()
        .find(|n| n.node_id == node_sha)
        .expect("node should exist");
    assert_eq!(node.body, "Secret claim text");

    // Resolve node_id to event SHA (same value for Say events)
    let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, &node_sha).unwrap();
    assert_eq!(resolved, node_sha);

    // Purge using the resolved event SHA
    let report = purge::purge_event(&git, &thread_id, &resolved).unwrap();
    assert_eq!(report.events_purged, 1);

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.nodes[0].body, "[purged]");
}

#[test]
fn purge_by_node_id_prefix_resolves_and_purges() {
    let (_repo, git) = setup();
    let (thread_id, node_sha) = make_thread_with_node(&git);

    // Use an 8-char prefix of the node ID
    let prefix = &node_sha[..8];
    let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, prefix).unwrap();
    assert_eq!(resolved, node_sha);

    let report = purge::purge_event(&git, &thread_id, &resolved).unwrap();
    assert_eq!(report.events_purged, 1);

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.nodes[0].body, "[purged]");
}

// ---- dry-run tests ----

#[test]
fn plan_purge_event_shows_target() {
    let (_repo, git) = setup();
    let (thread_id, node_sha) = make_thread_with_node(&git);

    let plan = purge::plan_purge_event(&git, &thread_id, &node_sha).unwrap();
    assert_eq!(plan.events.len(), 1);
    assert_eq!(plan.events[0].event_sha, node_sha);
    assert!(plan.events[0].has_body);
}

#[test]
fn plan_purge_actor_shows_all_matches() {
    let (_repo, git) = setup();
    let tid = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Issue",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    write_ops::say_node(
        &git,
        &tid,
        NodeType::Claim,
        "Claim 1",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    write_ops::say_node(
        &git,
        &tid,
        NodeType::Claim,
        "Claim 2",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let plan = purge::plan_purge_actor(&git, "human/bob").unwrap();
    assert_eq!(plan.events.len(), 2);
}
