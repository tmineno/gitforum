//! Module integration tests for `src/internal/thread.rs` and the
//! thread-creation surface in `src/internal/create.rs` —
//! replay, enumeration, creation, ID resolution, and import-path
//! timestamp overrides (test-policy.md category 1).

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::create;
use git_forum::internal::event::{self, ThreadKind};
use git_forum::internal::id_alloc;
use git_forum::internal::thread;

use support::forum::{
    fixed_clock, sample_create_event as sample_create, sample_state_event as sample_state, setup,
    test_thread_id,
};

// ---- Replay & enumeration ----

#[test]
fn replay_thread_from_git() {
    let (_repo, git, _paths) = setup();
    let tid = test_thread_id(ThreadKind::Rfc, 4);
    let create = sample_create(&tid, ThreadKind::Rfc, "Test RFC");
    let state_ev = sample_state(&tid, "proposed");
    event::write_event(&git, &create).unwrap();
    event::write_event(&git, &state_ev).unwrap();

    let state = thread::replay_thread(&git, &tid).unwrap();
    assert_eq!(state.id, tid);
    assert_eq!(state.kind, ThreadKind::Rfc);
    assert_eq!(state.title, "Test RFC");
    assert_eq!(state.status, "proposed");
    assert_eq!(state.created_by, "human/alice");
    assert_eq!(state.events.len(), 2);
}

#[test]
fn list_thread_ids_finds_stored_threads() {
    let (_repo, git, _paths) = setup();
    let ask_id = test_thread_id(ThreadKind::Issue, 5);
    let rfc_id = test_thread_id(ThreadKind::Rfc, 6);
    event::write_event(&git, &sample_create(&ask_id, ThreadKind::Issue, "Bug")).unwrap();
    event::write_event(&git, &sample_create(&rfc_id, ThreadKind::Rfc, "Proposal")).unwrap();

    let ids = thread::list_thread_ids(&git).unwrap();
    assert_eq!(ids.len(), 2);
    assert!(
        ids.iter().any(|id| id == &ask_id),
        "should contain ASK thread"
    );
    assert!(
        ids.iter().any(|id| id == &rfc_id),
        "should contain RFC thread"
    );
    // The fixture's `test_thread_id` helper still uses the legacy
    // kind-prefixed form to exercise the 1.x-on-disk path that
    // list_thread_ids must keep reading; assert any valid form.
    for id in &ids {
        assert!(
            id_alloc::is_valid_thread_id(id),
            "expected valid thread id, got: {id}"
        );
    }
}

#[test]
fn list_thread_ids_empty_repo() {
    let (_repo, git, _paths) = setup();
    let ids = thread::list_thread_ids(&git).unwrap();
    assert!(ids.is_empty());
}

// ---- Creation ----

#[test]
fn create_issue_returns_bare_token_id() {
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
    // SPEC-2.0 §6.2: 2.0 native creation produces bare 8-char base36 tokens.
    assert!(
        id_alloc::is_bare_token(&id),
        "expected bare token, got: {id}"
    );
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

// ---- Resolution ----

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
    let prefix = &id[..8];
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
    // 2.0 native IDs are bare tokens already; resolve must accept them
    // verbatim via the exact-match path.
    let resolved = thread::resolve_thread_id(&git, &id).unwrap();
    assert_eq!(resolved, id);
    // The leading-`@` display form must also resolve.
    let resolved_at = thread::resolve_thread_id(&git, &format!("@{id}")).unwrap();
    assert_eq!(resolved_at, id);
}

#[test]
fn resolve_thread_id_not_found() {
    let (_repo, git, _paths) = setup();
    let result = thread::resolve_thread_id(&git, "RFC-nonexist");
    assert!(result.is_err());
}
