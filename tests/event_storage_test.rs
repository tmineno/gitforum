//! Module integration tests for `src/internal/event.rs` storage path —
//! `write_event` / `read_event` / `load_thread_events`
//! (test-policy.md category 1).

mod support;

use git_forum::internal::event::{self, EventType, ThreadKind};

use support::forum::{
    sample_create_event as sample_create, sample_state_event as sample_state,
    setup_no_init as setup, test_thread_id,
};

#[test]
fn write_and_read_event_roundtrip() {
    let (_repo, git, _paths) = setup();
    let tid = test_thread_id(ThreadKind::Rfc, 1);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Test RFC");

    let commit_sha = event::write_event(&git, &ev).unwrap();
    assert!(!commit_sha.is_empty());

    let loaded = event::read_event(&git, &commit_sha).unwrap();
    assert_eq!(loaded.event_id, commit_sha);
    assert_eq!(loaded.event_type, EventType::Create);
    assert_eq!(loaded.thread_id, tid);
    assert_eq!(loaded.title.as_deref(), Some("Test RFC"));
    assert_eq!(loaded.kind, Some(ThreadKind::Rfc));
}

#[test]
fn write_two_events_creates_parent_chain() {
    let (_repo, git, _paths) = setup();
    let tid = test_thread_id(ThreadKind::Rfc, 2);
    let create = sample_create(&tid, ThreadKind::Rfc, "Test RFC");
    let state = sample_state(&tid, "proposed");

    let sha1 = event::write_event(&git, &create).unwrap();
    let sha2 = event::write_event(&git, &state).unwrap();
    assert_ne!(sha1, sha2);

    let shas = git.rev_list(&format!("refs/forum/threads/{tid}")).unwrap();
    assert_eq!(shas.len(), 2);
    assert_eq!(shas[0], sha2);
    assert_eq!(shas[1], sha1);
}

#[test]
fn load_thread_events_returns_chronological() {
    let (_repo, git, _paths) = setup();
    let tid = test_thread_id(ThreadKind::Rfc, 3);
    let create = sample_create(&tid, ThreadKind::Rfc, "Test RFC");
    let state = sample_state(&tid, "proposed");
    event::write_event(&git, &create).unwrap();
    event::write_event(&git, &state).unwrap();

    let events = event::load_thread_events(&git, &tid).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, EventType::Create);
    assert_eq!(events[1].event_type, EventType::State);
}
