//! Unit-level coverage for the `migrate_legacy_to_snapshot`
//! projection (task `9635buy0` step 3 / items 5 + 6).
//!
//! Two SPEC-3.0 §8 invariants under test:
//!
//! - Item 5 / §8.1 step 4: the projected snapshot's `status` is the
//!   target category's `initial_status` from the v3 built-in
//!   registry, NOT the replayed legacy final status.
//! - Item 6 / §8.3: tag augmentation consults the **legacy kind**
//!   (not just lifecycle), so `Issue → bug` and `Dec → decision`
//!   land on the projected `tags` even when the source chain has no
//!   `facet_set`.
//!
//! The walk + write side (`commands::migrate::run`) is wired in a
//! later step; this test calls the projection helper directly.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::commands::migrate;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::legacy::event::{self, Event, EventType, ThreadKind};

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

fn create_event(thread_id: &str, kind: ThreadKind, title: &str) -> Event {
    Event {
        thread_id: thread_id.into(),
        event_type: EventType::Create,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        actor: "human/alice".into(),
        title: Some(title.into()),
        kind: Some(kind),
        body: Some("body".into()),
        ..Event::default()
    }
}

fn state_event(thread_id: &str, new_state: &str, ts_offset_min: i64) -> Event {
    Event {
        thread_id: thread_id.into(),
        event_type: EventType::State,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
            + chrono::Duration::minutes(ts_offset_min),
        actor: "human/alice".into(),
        new_state: Some(new_state.into()),
        ..Event::default()
    }
}

fn build_chain(git: &GitOps, kind: ThreadKind, title: &str, tail: Vec<Event>) -> String {
    let id = id_alloc::alloc_thread_id_with_nonce(
        kind,
        "human/alice",
        title,
        "2026-01-01T00:00:00Z",
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );
    let create = create_event(&id, kind, title);
    event::write_event(git, &create).unwrap();
    for ev in tail {
        let mut ev = ev;
        ev.thread_id = id.clone();
        event::write_event(git, &ev).unwrap();
    }
    id
}

#[test]
fn projection_uses_target_category_initial_status_not_legacy_final() {
    // A v1 RFC closed in `accepted` state must project to `draft`
    // (the rfc category's initial_status). SPEC-3.0 §8.1 step 4:
    // migration is intentionally lossy on state. The legacy final
    // status survives as ancestor commits + legacy/events.ndjson.
    let (_repo, git, _paths) = setup();
    let id = build_chain(
        &git,
        ThreadKind::Rfc,
        "An accepted RFC",
        vec![state_event("PLACEHOLDER", "accepted", 1)],
    );

    let doc = migrate::migrate_legacy_to_snapshot(&git, &id).unwrap();
    assert_eq!(doc.snapshot.category, "rfc");
    assert_eq!(
        doc.snapshot.status, "draft",
        "v3 rfc.initial_status must override the legacy final status; got {}",
        doc.snapshot.status
    );
}

#[test]
fn projection_uses_task_initial_status_open_for_execution() {
    let (_repo, git, _paths) = setup();
    let id = build_chain(
        &git,
        ThreadKind::Task,
        "A closed task",
        vec![state_event("PLACEHOLDER", "closed", 1)],
    );

    let doc = migrate::migrate_legacy_to_snapshot(&git, &id).unwrap();
    assert_eq!(doc.snapshot.category, "task");
    assert_eq!(doc.snapshot.status, "open");
}

#[test]
fn projection_augments_tag_bug_for_issue_kind() {
    // SPEC-3.0 §8.3: `Issue` (covers v1 bug/issue/ASK-*) collapses
    // into category=task and MUST carry the `bug` tag so the
    // distinction survives the lifecycle merge.
    let (_repo, git, _paths) = setup();
    let id = build_chain(&git, ThreadKind::Issue, "A bug", vec![]);

    let doc = migrate::migrate_legacy_to_snapshot(&git, &id).unwrap();
    assert_eq!(doc.snapshot.category, "task");
    assert!(
        doc.snapshot.tags.iter().any(|t| t == "bug"),
        "issue kind must augment with `bug` tag, got tags={:?}",
        doc.snapshot.tags
    );
}

#[test]
fn projection_augments_tag_decision_for_dec_kind() {
    let (_repo, git, _paths) = setup();
    let id = build_chain(&git, ThreadKind::Dec, "A decision", vec![]);

    let doc = migrate::migrate_legacy_to_snapshot(&git, &id).unwrap();
    assert_eq!(doc.snapshot.category, "task");
    assert!(
        doc.snapshot.tags.iter().any(|t| t == "decision"),
        "dec kind must augment with `decision` tag, got tags={:?}",
        doc.snapshot.tags
    );
}

#[test]
fn projection_does_not_double_augment_tag_already_present() {
    // Idempotence: if the source chain already carries the canonical
    // tag (via a 2.0 facet_set, say), augmentation must not duplicate.
    let (_repo, git, _paths) = setup();
    let id = build_chain(&git, ThreadKind::Issue, "Already-tagged bug", vec![]);
    let facet = Event {
        thread_id: id.clone(),
        event_type: EventType::FacetSet,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        actor: "system/migrate".into(),
        lifecycle: Some("execution".into()),
        tags_add: vec!["bug".into()],
        ..Event::default()
    };
    event::write_event(&git, &facet).unwrap();

    let doc = migrate::migrate_legacy_to_snapshot(&git, &id).unwrap();
    let bug_count = doc.snapshot.tags.iter().filter(|t| *t == "bug").count();
    assert_eq!(
        bug_count, 1,
        "augmentation must be idempotent; got tags={:?}",
        doc.snapshot.tags
    );
}

#[test]
fn projection_rfc_kind_carries_no_canonical_extra_tag() {
    // Rfc is its own category — no extra augmentation needed.
    let (_repo, git, _paths) = setup();
    let id = build_chain(&git, ThreadKind::Rfc, "Plain RFC", vec![]);

    let doc = migrate::migrate_legacy_to_snapshot(&git, &id).unwrap();
    assert_eq!(doc.snapshot.category, "rfc");
    assert!(
        !doc.snapshot
            .tags
            .iter()
            .any(|t| t == "bug" || t == "decision"),
        "rfc kind must not gain bug/decision augmentation, got tags={:?}",
        doc.snapshot.tags
    );
}
