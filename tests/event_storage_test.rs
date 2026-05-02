//! Module integration tests for `src/internal/event.rs` storage path —
//! `write_event` / `read_event` / `load_thread_events`
//! (test-policy.md category 1).

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::event::{self, Event, EventType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    (repo, git, paths)
}

fn test_thread_id(kind: ThreadKind, seed: u8) -> String {
    id_alloc::alloc_thread_id_with_nonce(
        kind,
        "human/alice",
        "test",
        "2026-01-01T00:00:00Z",
        &[seed, seed, seed, seed, seed, seed, seed, seed],
    )
}

fn sample_create(thread_id: &str, kind: ThreadKind, title: &str) -> Event {
    Event {
        event_id: "evt-0001".into(),
        thread_id: thread_id.into(),
        event_type: EventType::Create,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        actor: "human/alice".into(),
        title: Some(title.into()),
        kind: Some(kind),
        ..Event::default()
    }
}

fn sample_state(thread_id: &str, new_state: &str) -> Event {
    Event {
        event_id: "evt-0002".into(),
        thread_id: thread_id.into(),
        event_type: EventType::State,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        actor: "human/bob".into(),
        new_state: Some(new_state.into()),
        ..Event::default()
    }
}

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
