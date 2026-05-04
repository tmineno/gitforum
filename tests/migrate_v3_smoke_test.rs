//! Smoke test for the new `migrate --to 3.0` walk (task `9635buy0`
//! step 3b). End-to-end: build a v1 legacy chain at a real ref, run
//! migrate, assert the ref tip is now a 3.0 snapshot tree with
//! `legacy/events.ndjson` written.
//!
//! Comprehensive validity coverage (all four §8.3 source kinds, 1.x
//! and 2.0 fixtures, dry-run, idempotence, error reporting) is
//! task step 7 / item 14 in `tests/migrate_validity_test.rs`.

mod support;

use std::process::Command;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::event::{self, Event, EventType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::snapshot;

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

#[test]
fn migrate_walk_rewrites_legacy_chain_to_snapshot_with_archive() {
    let (repo, git, _paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Issue,
        "human/alice",
        "Smoke",
        "2026-01-01T00:00:00Z",
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );
    event::write_event(&git, &create_event(&id, ThreadKind::Issue, "Smoke")).unwrap();

    // Pre-migration: read_snapshot must reject as LegacyEventChain.
    let pre = snapshot::read_snapshot(&git, &id).unwrap_err();
    assert!(
        matches!(
            pre,
            git_forum::internal::error::ForumError::LegacyEventChain
        ),
        "pre-migrate read must fail with LegacyEventChain, got {pre:?}"
    );

    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .expect("git-forum migrate should run");
    assert!(
        out.status.success(),
        "migrate exited non-zero:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[migrated]"),
        "no [migrated] line in:\n{stdout}"
    );

    // Post-migration: read_snapshot must succeed.
    let doc = snapshot::read_snapshot(&git, &id)
        .expect("post-migrate read_snapshot must return a 3.0 snapshot");
    assert_eq!(doc.snapshot.category, "task", "issue → task per §8.3");
    assert_eq!(
        doc.snapshot.status, "open",
        "task initial_status must be `open`"
    );
    assert!(
        doc.snapshot.tags.iter().any(|t| t == "bug"),
        "issue kind must augment with `bug` tag, got {:?}",
        doc.snapshot.tags
    );

    // Archive must be written.
    let tip = git
        .run(&["rev-parse", &format!("refs/forum/threads/{id}")])
        .unwrap();
    let entries = git
        .run(&["ls-tree", "-r", "--name-only", tip.trim()])
        .unwrap();
    assert!(
        entries.lines().any(|l| l == "legacy/events.ndjson"),
        "legacy/events.ndjson missing from migrated tree:\n{entries}"
    );
}

#[test]
fn migrate_skips_already_3_0_threads() {
    // A v3-native thread (thread.toml at tip, no event.json) must
    // be reported as already-migrated and NOT rewritten.
    let (repo, _git, _paths) = setup();
    let bin = env!("CARGO_BIN_EXE_git-forum");

    let create_out = Command::new(bin)
        .current_dir(repo.path())
        .args([
            "new",
            "rfc",
            "Native 3.0",
            "--body",
            "rfc body",
            "--as",
            "human/alice",
        ])
        .output()
        .unwrap();
    assert!(
        create_out.status.success(),
        "git-forum new rfc failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&create_out.stdout),
        String::from_utf8_lossy(&create_out.stderr),
    );

    let migrate_out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .unwrap();
    assert!(migrate_out.status.success());
    let stdout = String::from_utf8_lossy(&migrate_out.stdout);
    assert!(
        stdout.contains("[skip]"),
        "v3-native thread should be reported as skip:\n{stdout}"
    );
    assert!(
        !stdout.contains("[migrated]"),
        "v3-native thread must not be migrated:\n{stdout}"
    );
}

#[test]
fn migrate_pinned_write_rejects_concurrent_event() {
    // Regression test for objection `e630f01f` (task `9635buy0`):
    // simulate a concurrent legacy event landing between the pin
    // capture (`load_thread_events_at` reads from the captured tip)
    // and the snapshot write. The CAS in
    // `write_snapshot_with_archive_pinned` MUST reject the second
    // write rather than silently overwrite — better to fail loud
    // than to commit a snapshot whose archive is missing the
    // racer's event.
    use git_forum::internal::commands::migrate;
    use git_forum::internal::error::ForumError;
    use git_forum::internal::refs;
    use git_forum::internal::snapshot;

    let (_repo, git, _paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Issue,
        "human/alice",
        "Race",
        "2026-01-01T00:00:00Z",
        &[2, 2, 2, 2, 2, 2, 2, 2],
    );
    event::write_event(&git, &create_event(&id, ThreadKind::Issue, "Race")).unwrap();

    // Capture the pin (mimics what migrate_one does).
    let refname = refs::thread_ref(&id);
    let expected_tip = git.resolve_ref(&refname).unwrap().unwrap();

    // Project from the pinned tip — at this point the tip matches.
    let events = event::load_thread_events_at(&git, &expected_tip).unwrap();
    let archive_bytes = events
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let doc = migrate::migrate_legacy_to_snapshot_at(&git, &id, &expected_tip).unwrap();

    // Simulate a concurrent legacy event landing AFTER the pin.
    let racer = Event {
        thread_id: id.clone(),
        event_type: EventType::Say,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 5, 0).unwrap(),
        actor: "human/bob".into(),
        body: Some("a comment that races migration".into()),
        node_type: Some(git_forum::internal::event::NodeType::Comment),
        ..Event::default()
    };
    event::write_event(&git, &racer).unwrap();

    // Now try the pinned write. CAS expected to reject because the
    // ref is at the racer's commit, not at expected_tip.
    let result = snapshot::write_snapshot_with_archive_pinned(
        &git,
        &id,
        &doc,
        "migrate (should fail under race)",
        archive_bytes.as_bytes(),
        &expected_tip,
    );
    match result {
        Err(ForumError::SnapshotWriteConflict(_)) => {
            // expected
        }
        Err(other) => panic!("expected SnapshotWriteConflict, got {other:?}"),
        Ok(_) => panic!(
            "pinned write must NOT silently overwrite a concurrent legacy event \
             (objection e630f01f); the snapshot would be missing the racer's event"
        ),
    }

    // Sanity: the racer event is still readable on the chain.
    let events_after = event::load_thread_events(&git, &id).unwrap();
    assert_eq!(events_after.len(), 2, "racer event should be present");
}
