mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
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

// ---- ID allocation ----

#[test]
fn alloc_issue_id_has_correct_prefix() {
    let id = id_alloc::alloc_thread_id(ThreadKind::Issue, "human/alice", "Bug", "2026-01-01T00:00:00Z");
    assert!(id.starts_with("ASK-"), "expected ASK- prefix, got: {id}");
    let token = &id[4..];
    assert_eq!(token.len(), 8, "token length should be 8, got: {}", token.len());
    assert!(
        token.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
        "token has invalid chars: {token}"
    );
}

#[test]
fn alloc_ids_are_unique_with_different_nonces() {
    let id1 = id_alloc::alloc_thread_id(ThreadKind::Rfc, "human/alice", "Title", "2026-01-01T00:00:00Z");
    let id2 = id_alloc::alloc_thread_id(ThreadKind::Rfc, "human/alice", "Title", "2026-01-01T00:00:00Z");
    assert_ne!(id1, id2, "two allocations with random nonces should differ");
}

#[test]
fn alloc_deterministic_with_nonce() {
    let id1 = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc,
        "human/alice",
        "Test",
        "2026-01-01",
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );
    let id2 = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc,
        "human/alice",
        "Test",
        "2026-01-01",
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );
    assert_eq!(id1, id2, "same inputs + nonce should produce same ID");
}

// ---- ID format validation ----

#[test]
fn is_valid_thread_id_both_formats() {
    assert!(id_alloc::is_valid_thread_id("RFC-0001"));
    assert!(id_alloc::is_valid_thread_id("ASK-0042"));
    assert!(id_alloc::is_valid_thread_id("RFC-a7f3b2x1"));
    assert!(id_alloc::is_valid_thread_id("JOB-x8n2q1d4"));
    assert!(!id_alloc::is_valid_thread_id("UNKNOWN-0001"));
    assert!(!id_alloc::is_valid_thread_id("garbage"));
}

// ---- Thread creation ----

#[test]
fn create_issue_returns_opaque_id() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "First issue",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    assert!(id.starts_with("ASK-"), "got: {id}");
    assert!(id_alloc::is_opaque_id(&id), "expected opaque ID, got: {id}");
}

#[test]
fn create_rfc_initial_status_is_draft() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "First RFC",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "draft");
    assert_eq!(state.kind, ThreadKind::Rfc);
    assert_eq!(state.title, "First RFC");
}

#[test]
fn create_multiple_threads_of_same_kind() {
    let (_repo, git, _paths) = setup();
    let id1 = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "RFC A",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let id2 = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "RFC B",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let id3 = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "RFC C",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    let all = thread::list_thread_ids(&git).unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.contains(&id1));
    assert!(all.contains(&id2));
    assert!(all.contains(&id3));
}

// ---- Thread resolution ----

#[test]
fn resolve_thread_id_exact_match() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let resolved = thread::resolve_thread_id(&git, &id).unwrap();
    assert_eq!(resolved, id);
}

#[test]
fn resolve_thread_id_prefix_match() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    // Use KIND + first 4 chars of token as prefix
    let prefix = &id[..8]; // "RFC-XXXX"
    let resolved = thread::resolve_thread_id(&git, prefix).unwrap();
    assert_eq!(resolved, id);
}

#[test]
fn resolve_thread_id_token_only() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let token = &id[4..]; // just the 8-char token
    let resolved = thread::resolve_thread_id(&git, token).unwrap();
    assert_eq!(resolved, id);
}

#[test]
fn resolve_thread_id_not_found() {
    let (_repo, git, _paths) = setup();
    let result = thread::resolve_thread_id(&git, "RFC-nonexist");
    assert!(result.is_err());
}

// ---- ls ----

#[test]
fn ls_shows_all_kinds() {
    let (_repo, git, _paths) = setup();
    let ask_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Bug",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let rfc_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Proposal",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let all_ids = thread::list_thread_ids(&git).unwrap();
    let mut states = Vec::new();
    for id in &all_ids {
        states.push(thread::replay_thread(&git, id).unwrap());
    }
    let refs: Vec<&thread::ThreadState> = states.iter().collect();
    let out = show::render_ls(&refs);
    assert!(out.contains(&ask_id));
    assert!(out.contains(&rfc_id));
    assert!(out.contains("Bug"));
    assert!(out.contains("Proposal"));
}

#[test]
fn ls_filtered_by_kind() {
    let (_repo, git, _paths) = setup();
    let _ask_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Bug",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let rfc_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Proposal",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let all_ids = thread::list_thread_ids(&git).unwrap();
    let mut rfc_states = Vec::new();
    for id in &all_ids {
        let s = thread::replay_thread(&git, id).unwrap();
        if s.kind == ThreadKind::Rfc {
            rfc_states.push(s);
        }
    }
    let refs: Vec<&thread::ThreadState> = rfc_states.iter().collect();
    let out = show::render_ls(&refs);
    assert!(out.contains(&rfc_id));
    assert!(out.contains("Proposal"));
}

// ---- show ----

#[test]
fn show_contains_all_required_fields() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Initial thread body.\nSecond line."),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    let out = show::render_show(&state, false);

    assert!(out.contains(&id), "missing thread id");
    assert!(out.contains("Test RFC"), "missing title");
    assert!(out.contains("rfc"), "missing kind");
    assert!(out.contains("draft"), "missing status");
    assert!(out.contains("human/alice"), "missing actor");
    assert!(out.contains("body:"), "missing body section");
    assert!(out.contains("Initial thread body."), "missing body content");
    assert!(
        out.contains("Second line."),
        "missing multiline body content"
    );
    assert!(out.contains("2026-01-01T00:00:00Z"), "missing timestamp");
    assert!(out.contains("timeline:"), "missing timeline section");
    assert!(out.contains("date"), "missing timeline header");
    assert!(out.contains("node_id"), "missing node_id timeline header");
    assert!(out.contains("event_id"), "missing event_id timeline header");
    let event_id = &state.events[0].event_id;
    assert!(
        out.contains(&event_id[..event_id.len().min(16)]),
        "missing create event id in timeline"
    );
    assert!(
        out.contains("create"),
        "missing create event type in timeline"
    );
}

#[test]
fn show_replay_consistency() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state1 = thread::replay_thread(&git, &id).unwrap();
    let state2 = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(
        show::render_show(&state1, false),
        show::render_show(&state2, false)
    );
}

#[test]
fn show_snapshot_contains_expected_fields() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Initial thread body."),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    let out = show::render_show(&state, false);

    assert!(out.contains(&id));
    assert!(out.contains("Test RFC"));
    assert!(out.contains("rfc"));
    assert!(out.contains("draft"));
    assert!(out.contains("Initial thread body."));
    assert!(out.contains("human/alice"));
    assert!(out.contains("2026-01-01T00:00:00Z"));
    assert!(out.contains("create"));
}

#[test]
fn create_thread_body_roundtrips_in_replay() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Problem statement and context."),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(
        state.body.as_deref(),
        Some("Problem statement and context.")
    );
}

// ---- timestamp override (ISSUE-0100) ----

#[test]
fn create_thread_with_timestamp_uses_override() {
    let (_repo, git, _paths) = setup();
    let custom_ts = Utc.with_ymd_and_hms(2020, 6, 15, 12, 30, 0).unwrap();
    let id = create::create_thread_with_timestamp(
        &git,
        ThreadKind::Issue,
        "Imported issue",
        Some("Body from import"),
        None,
        "human/alice",
        &fixed_clock(),
        custom_ts,
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.events[0].created_at, custom_ts);
    assert_eq!(state.created_at, custom_ts);
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
