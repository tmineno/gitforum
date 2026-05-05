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
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::legacy::event::{self, Event, EventType, ThreadKind};
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
        ThreadKind::Issue.id_prefix(),
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
        ThreadKind::Issue.id_prefix(),
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
        node_type: Some(git_forum::internal::legacy::event::NodeType::Comment),
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

// ---------------------------------------------------------------
// Strict / reporting replay (task `9635buy0` step 4 / item 7).
// Migration uses strict replay so malformed legacy events surface
// instead of being silently dropped, but missing facet_set on 1.x
// chains is NOT an error.
// ---------------------------------------------------------------

fn say_event(thread_id: &str, body: &str, ts_offset_min: i64) -> Event {
    Event {
        thread_id: thread_id.into(),
        event_type: EventType::Say,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
            + chrono::Duration::minutes(ts_offset_min),
        actor: "human/bob".into(),
        body: Some(body.into()),
        node_type: Some(git_forum::internal::legacy::event::NodeType::Comment),
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

#[test]
fn migrate_strict_surfaces_invalid_state_event_as_warning() {
    // SPEC-3.0 §8 + task item 7: a malformed legacy event (here, a
    // state event with an unparseable new_state) does NOT abort
    // migration, but it MUST surface as a warning instead of being
    // silently dropped.
    use std::process::Command;
    let (repo, git, _paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc.id_prefix(),
        "human/alice",
        "Strict",
        "2026-01-01T00:00:00Z",
        &[3, 3, 3, 3, 3, 3, 3, 3],
    );
    event::write_event(&git, &create_event(&id, ThreadKind::Rfc, "Strict")).unwrap();
    // `xyz-not-a-state` parses neither as canonical 3.0 nor as any
    // 1.x synonym → InvalidStateValue under strict replay.
    event::write_event(&git, &state_event(&id, "xyz-not-a-state", 1)).unwrap();

    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .expect("git-forum migrate should run");
    assert!(
        out.status.success(),
        "migration MUST succeed even with a malformed event;\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning:") && stderr.contains(&id) && stderr.contains("xyz-not-a-state"),
        "strict replay must surface the malformed state value as a warning:\n{stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[migrated]"),
        "thread should still appear as migrated:\n{stdout}"
    );
}

#[test]
fn migrate_with_missing_facet_set_succeeds_for_1x_chain() {
    // Task item 7 / objection `efe64dba`: a 1.x chain has no
    // facet_set event by design. Migration MUST succeed and infer
    // category/tags from the legacy kind. The surface is an
    // informational `note:` line, NOT a `warning:` line.
    use std::process::Command;
    let (repo, git, _paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Issue.id_prefix(),
        "human/alice",
        "Pre-2.0",
        "2026-01-01T00:00:00Z",
        &[4, 4, 4, 4, 4, 4, 4, 4],
    );
    // Pure 1.x chain: just a create event + a comment. No facet_set
    // anywhere — `state.lifecycle_explicit` stays false.
    event::write_event(&git, &create_event(&id, ThreadKind::Issue, "Pre-2.0")).unwrap();
    event::write_event(&git, &say_event(&id, "a comment", 1)).unwrap();

    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .expect("git-forum migrate should run");
    assert!(
        out.status.success(),
        "1.x chain (no facet_set) MUST migrate successfully;\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("note:") && stderr.contains(&id) && stderr.contains("inferred"),
        "missing facet_set should produce an inferred-metadata `note:`:\n{stderr}"
    );
    assert!(
        !stderr.contains("warning:"),
        "missing facet_set must NOT be surfaced as a warning (objection efe64dba):\n{stderr}"
    );

    // Sanity: the projected snapshot has the expected category/tags.
    let doc = git_forum::internal::snapshot::read_snapshot(&git, &id)
        .expect("post-migrate read_snapshot must succeed");
    assert_eq!(doc.snapshot.category, "task");
    assert_eq!(doc.snapshot.status, "open");
    assert!(doc.snapshot.tags.iter().any(|t| t == "bug"));
}

// ---------------------------------------------------------------
// --dry-run (task `9635buy0` step 5 / item 15).
// ---------------------------------------------------------------

#[test]
fn migrate_dry_run_does_not_advance_the_ref_tip() {
    // SPEC-3.0 §8.1: `git forum migrate --to 3.0 --dry-run` reports
    // the planned work without writing anything. The ref tip OID
    // MUST be byte-identical before and after.
    let (repo, git, _paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc.id_prefix(),
        "human/alice",
        "DryRun",
        "2026-01-01T00:00:00Z",
        &[5, 5, 5, 5, 5, 5, 5, 5],
    );
    event::write_event(&git, &create_event(&id, ThreadKind::Rfc, "DryRun")).unwrap();

    let refname = format!("refs/forum/threads/{id}");
    let before = git
        .resolve_ref(&refname)
        .unwrap()
        .expect("ref must exist after the legacy write");

    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0", "--dry-run"])
        .output()
        .expect("git-forum migrate --dry-run should run");
    assert!(
        out.status.success(),
        "dry-run exited non-zero:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[DRY-RUN]"),
        "dry-run output must be flagged as such:\n{stdout}"
    );
    assert!(
        stdout.contains("[plan]") && stdout.contains(&id),
        "dry-run must enumerate planned migrations:\n{stdout}"
    );
    assert!(
        !stdout.contains("[migrated]"),
        "dry-run must NOT record any actual migration:\n{stdout}"
    );

    // The critical invariant: ref OID unchanged.
    let after = git.resolve_ref(&refname).unwrap().unwrap();
    assert_eq!(
        before, after,
        "ref tip OID changed during --dry-run (before={before}, after={after})"
    );

    // And the tip is still legacy — read_snapshot still rejects.
    let probe = snapshot::read_snapshot(&git, &id).unwrap_err();
    assert!(
        matches!(
            probe,
            git_forum::internal::error::ForumError::LegacyEventChain
        ),
        "post-dry-run read_snapshot must still return LegacyEventChain, got {probe:?}"
    );
}

#[test]
fn migrate_dry_run_followed_by_real_run_writes_snapshot() {
    // Sanity: --dry-run is repeatable and does not poison the
    // subsequent real run.
    let (repo, git, _paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Issue.id_prefix(),
        "human/alice",
        "PlanThenGo",
        "2026-01-01T00:00:00Z",
        &[6, 6, 6, 6, 6, 6, 6, 6],
    );
    event::write_event(&git, &create_event(&id, ThreadKind::Issue, "PlanThenGo")).unwrap();

    let bin = env!("CARGO_BIN_EXE_git-forum");

    let dry = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0", "--dry-run"])
        .output()
        .unwrap();
    assert!(dry.status.success());
    assert!(snapshot::read_snapshot(&git, &id).is_err()); // still legacy

    let real = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .unwrap();
    assert!(real.status.success());
    let stdout = String::from_utf8_lossy(&real.stdout);
    assert!(stdout.contains("[migrated]"));

    let doc = snapshot::read_snapshot(&git, &id).expect("real run must complete the migration");
    assert_eq!(doc.snapshot.category, "task");
}

// ---------------------------------------------------------------
// Structured migration report (task `9635buy0` step 6, items 9-10,
// 10a, 16, 17, 20).
// ---------------------------------------------------------------

#[test]
fn migrate_writes_machine_readable_report_under_git_dir() {
    // Item 10 / 20: report lives at `.git/forum/migration-report.json`,
    // never in the working tree.
    let (repo, _git, paths) = setup();
    // Build one legacy thread + one v3-native thread so the report
    // exercises both Migrated and AlreadyMigrated outcomes.
    let legacy_id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Issue.id_prefix(),
        "human/alice",
        "Legacy",
        "2026-01-01T00:00:00Z",
        &[7, 7, 7, 7, 7, 7, 7, 7],
    );
    event::write_event(
        &_git,
        &create_event(&legacy_id, ThreadKind::Issue, "Legacy"),
    )
    .unwrap();

    let bin = env!("CARGO_BIN_EXE_git-forum");
    Command::new(bin)
        .current_dir(repo.path())
        .args([
            "new",
            "rfc",
            "Native",
            "--body",
            "rfc body",
            "--as",
            "human/alice",
        ])
        .output()
        .unwrap();

    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .unwrap();
    assert!(out.status.success());

    let report_path = paths.git_forum.join("migration-report.json");
    assert!(
        report_path.exists(),
        "report file MUST be written at {} (SPEC-3.0 §4.3 local clone state)",
        report_path.display()
    );
    // Critical: report MUST NOT land in the working tree under `.forum/`.
    assert!(
        !paths.dot_forum.join("migration-report.json").exists(),
        "report MUST NOT be written under .forum/ (working tree)"
    );

    let body = std::fs::read_to_string(&report_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).expect("report must be valid JSON");
    assert!(json.get("generated_at").is_some(), "missing generated_at");
    assert_eq!(json["dry_run"], false);
    let threads = json["threads"]
        .as_array()
        .expect("threads must be an array");
    assert_eq!(threads.len(), 2, "expected two thread reports");

    let legacy_entry = threads
        .iter()
        .find(|t| t["thread_id"] == legacy_id)
        .expect("legacy thread missing from report");
    assert_eq!(legacy_entry["outcome"], "migrated");
    assert!(
        legacy_entry["archived_events"].as_u64().unwrap() >= 1,
        "archived_events should be set on migrated entry"
    );

    let native_entry = threads
        .iter()
        .find(|t| t["outcome"] == "already_migrated")
        .expect("native thread should be already_migrated");
    assert!(native_entry["thread_id"].as_str().unwrap() != legacy_id);
}

#[test]
fn migrate_is_idempotent_second_run_is_all_already_migrated() {
    // Item 16: running migrate on a repo that's already 3.0 is a
    // no-op. Every ref reports already_migrated; no new commits;
    // exit 0.
    let (repo, git, _paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc.id_prefix(),
        "human/alice",
        "Idem",
        "2026-01-01T00:00:00Z",
        &[8, 8, 8, 8, 8, 8, 8, 8],
    );
    event::write_event(&git, &create_event(&id, ThreadKind::Rfc, "Idem")).unwrap();

    let bin = env!("CARGO_BIN_EXE_git-forum");
    let first = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .unwrap();
    assert!(first.status.success());
    let tip_after_first = git
        .run(&["rev-parse", &format!("refs/forum/threads/{id}")])
        .unwrap()
        .trim()
        .to_string();

    let second = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "second migrate should be a clean no-op:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("[skip]") && !stdout.contains("[migrated]"),
        "second run must show skip for the migrated thread:\n{stdout}"
    );
    let tip_after_second = git
        .run(&["rev-parse", &format!("refs/forum/threads/{id}")])
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        tip_after_first, tip_after_second,
        "second migrate must not advance the ref tip"
    );
}

#[test]
fn migrate_strict_issue_records_omission_in_report() {
    // Item 17: a malformed legacy event surfaces in the report's
    // omissions list. Outcome is still `migrated` (strict-replay
    // issues don't fail the thread — only unrecoverable read/
    // project/write errors do, per item 10).
    let (repo, git, paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc.id_prefix(),
        "human/alice",
        "Strict",
        "2026-01-01T00:00:00Z",
        &[9, 9, 9, 9, 9, 9, 9, 9],
    );
    event::write_event(&git, &create_event(&id, ThreadKind::Rfc, "Strict")).unwrap();
    event::write_event(&git, &state_event(&id, "xyz-not-a-state", 1)).unwrap();

    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .unwrap();
    assert!(out.status.success(), "strict issues do not fail the run");

    let body = std::fs::read_to_string(paths.git_forum.join("migration-report.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let entry = json["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["thread_id"] == id)
        .unwrap();
    assert_eq!(entry["outcome"], "migrated");
    let omissions = entry["omissions"]
        .as_array()
        .expect("omissions array must exist for a thread with strict issues");
    assert!(
        omissions
            .iter()
            .any(|o| o["kind"] == "state" && o["item"] == "xyz-not-a-state"),
        "expected a state omission for `xyz-not-a-state`, got {omissions:?}"
    );
}

#[test]
fn migrate_unparseable_event_payload_records_error_outcome() {
    // Item 10 / 17: a ref whose chain has an unparseable event
    // payload becomes `outcome = error` and the run exits non-zero.
    // Other refs still complete.
    let (repo, git, paths) = setup();
    // A healthy thread that should still migrate cleanly.
    let healthy_id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc.id_prefix(),
        "human/alice",
        "Healthy",
        "2026-01-01T00:00:00Z",
        &[10, 10, 10, 10, 10, 10, 10, 10],
    );
    event::write_event(&git, &create_event(&healthy_id, ThreadKind::Rfc, "Healthy")).unwrap();

    // Hand-craft a ref whose tip commit has an `event.json` blob
    // containing garbage JSON. `read_event` will fail on it.
    let blob = git
        .hash_object(b"this is not valid json at all }{")
        .unwrap();
    let tree = git.mktree_single("event.json", &blob).unwrap();
    let commit = git.commit_tree(&tree, &[], "garbage event").unwrap();
    let bad_id = "badbadbad";
    git.create_ref(&format!("refs/forum/threads/{bad_id}"), &commit)
        .unwrap();

    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "migrate must exit non-zero when any thread's outcome is error"
    );

    let body = std::fs::read_to_string(paths.git_forum.join("migration-report.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let bad_entry = json["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["thread_id"] == bad_id)
        .expect("bad thread missing from report");
    assert_eq!(bad_entry["outcome"], "error");
    assert!(
        bad_entry["error"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "error string must be populated"
    );
    // The healthy thread must still have completed.
    let healthy_entry = json["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["thread_id"] == healthy_id)
        .unwrap();
    assert_eq!(healthy_entry["outcome"], "migrated");
}

#[test]
fn migrate_handles_legacy_approval_node_ids_with_slashes() {
    // Real-world regression (caught by running `git forum migrate
    // --to 3.0` against the live `gitforum-v2.0.2-refactor` repo,
    // 17 of 299 threads failed): legacy approval nodes have
    // `<event_sha>#<actor_id>` IDs and `actor_id` commonly contains
    // a namespace separator (`human/bob`). `git mktree`
    // rejects path components with `/`, so the projection MUST
    // sanitize before writing `nodes/<id>.toml`.
    use git_forum::internal::legacy::event::{Approval, ApprovalMechanism};
    let (repo, git, _paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc.id_prefix(),
        "human/alice",
        "ApprovalSlash",
        "2026-01-01T00:00:00Z",
        &[11, 11, 11, 11, 11, 11, 11, 11],
    );
    event::write_event(&git, &create_event(&id, ThreadKind::Rfc, "ApprovalSlash")).unwrap();
    // SPEC-2.0 §2.8: 1.x state events carried approvals inline.
    // Replay synthesizes one Approval node per `(event_sha, actor)`
    // pair as `<event_sha>#<actor_id>`. With actor=`human/bob`
    // the legacy node_id contains a `/`, which is what triggered
    // the real-world mktree rejection.
    let state_with_approval = Event {
        thread_id: id.clone(),
        event_type: EventType::State,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        actor: "human/alice".into(),
        new_state: Some("open".into()),
        approvals: vec![Approval {
            actor_id: "human/bob".into(),
            approved_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
            mechanism: ApprovalMechanism::Recorded,
            key_id: None,
            proof_ref: None,
        }],
        ..Event::default()
    };
    event::write_event(&git, &state_with_approval).unwrap();

    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "migration must succeed even when an approval's actor_id contains `/`:\n\
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[migrated]"), "thread should migrate");

    // The approval node should be present with a tree-safe ID
    // (slashes scrubbed to `-`).
    let doc = snapshot::read_snapshot(&git, &id).expect("post-migrate read_snapshot");
    let approval = doc
        .nodes
        .iter()
        .find(|n| matches!(n.record.kind, git_forum::internal::node::NodeKind::Approval))
        .expect("approval node missing from migrated snapshot");
    assert!(
        !approval.record.id.contains('/'),
        "migrated approval node id must not contain `/`, got `{}`",
        approval.record.id
    );
    assert!(
        approval.record.id.contains("human-bob"),
        "approval id should preserve the actor (with `/` → `-`), got `{}`",
        approval.record.id
    );
}

#[test]
fn migrate_drops_invalid_tags_and_records_them_in_report() {
    // Task `9635buy0` objection `e285682f`: migration MUST validate
    // every projected tag against SPEC-3.0 §2.4 grammar. Invalid
    // tags are dropped from the snapshot and recorded as
    // `kind: "tag"` omissions in the per-thread report. Without
    // this gate, a v2 chain with a 1-char tag (legal under earlier
    // loose validators) would produce a 3.0 snapshot that violates
    // its own tag grammar.
    let (repo, git, paths) = setup();
    let id = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Issue.id_prefix(),
        "human/alice",
        "InvalidTag",
        "2026-01-01T00:00:00Z",
        &[12, 12, 12, 12, 12, 12, 12, 12],
    );
    event::write_event(&git, &create_event(&id, ThreadKind::Issue, "InvalidTag")).unwrap();
    // facet_set adds a mix of valid + invalid tags. Per §2.4:
    // - "x" — too short (length must be 2..=32)
    // - "all" — reserved literal
    // - "Bug" — uppercase; first char must be lowercase
    // - "needs-review" — valid
    let facet = Event {
        thread_id: id.clone(),
        event_type: EventType::FacetSet,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        actor: "system/migrate".into(),
        lifecycle: Some("execution".into()),
        tags_add: vec![
            "x".into(),
            "all".into(),
            "Bug".into(),
            "needs-review".into(),
        ],
        ..Event::default()
    };
    event::write_event(&git, &facet).unwrap();

    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "invalid tags should not fail migration"
    );

    // Snapshot must NOT carry the invalid tags.
    let doc = snapshot::read_snapshot(&git, &id).unwrap();
    assert!(
        doc.snapshot.tags.iter().any(|t| t == "needs-review"),
        "valid tag should survive, got {:?}",
        doc.snapshot.tags
    );
    for bad in ["x", "all", "Bug"] {
        assert!(
            !doc.snapshot.tags.iter().any(|t| t == bad),
            "invalid tag `{bad}` must be dropped, got {:?}",
            doc.snapshot.tags
        );
    }

    // Report must list each invalid tag as a `kind: "tag"` omission.
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(paths.git_forum.join("migration-report.json")).unwrap(),
    )
    .unwrap();
    let entry = report["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["thread_id"] == id)
        .unwrap();
    let omissions = entry["omissions"]
        .as_array()
        .expect("omissions array must be present");
    let tag_omissions: Vec<&str> = omissions
        .iter()
        .filter(|o| o["kind"] == "tag")
        .filter_map(|o| o["item"].as_str())
        .collect();
    for bad in ["x", "all", "Bug"] {
        assert!(
            tag_omissions.contains(&bad),
            "tag omission for `{bad}` missing from report; got {tag_omissions:?}"
        );
    }
}

#[test]
fn migrate_handles_mixed_chain_with_snapshot_bottom_and_event_tail() {
    // Task `9635buy0` objection `bf678561`: a Phase-2 cutover
    // produced refs whose tip is an event commit but whose ancestor
    // is a SPEC-3.0 snapshot commit. `read_snapshot` checks only the
    // tip tree, so it routes such refs to migration as
    // `LegacyEventChain`. The strict pinned reader MUST mirror
    // `replay_thread_at`'s mixed-chain walk: seed from the snapshot
    // ancestor and apply the event tail. A pure-event chain still
    // routes through `replay_strict`.
    use git_forum::internal::commands::migrate;
    use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
    use git_forum::internal::snapshot::{NodeWithBody, ThreadDocument};
    use git_forum::internal::thread::ThreadSnapshot;

    let (_repo, git, _paths) = setup();
    let id = "mixchain";
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

    // Bottom commit: a SPEC-3.0 snapshot at the ref.
    let mut doc = ThreadDocument::new(ThreadSnapshot {
        schema_version: ThreadSnapshot::SCHEMA_VERSION,
        id: id.into(),
        title: "Mixed".into(),
        category: "rfc".into(),
        status: "draft".into(),
        tags: vec!["pre-existing".into()],
        created_at: now,
        created_by: "human/alice".into(),
        updated_at: now,
        updated_by: "human/alice".into(),
        branch: None,
        supersedes: vec![],
    });
    // Add a node so the tail's reply can target it (not used here
    // but exercises the snapshot-seed path).
    doc.nodes.push(NodeWithBody {
        record: NodeRecord {
            id: "seedcomment".into(),
            kind: NodeKind::Comment,
            status: NodeStatus::Open,
            created_at: now,
            created_by: "human/alice".into(),
            updated_at: None,
            updated_by: None,
            reply_to: None,
            legacy_label: None,
        },
        body: "seeded".into(),
    });
    snapshot::write_snapshot(&git, id, &doc, "create snapshot").unwrap();

    // Tail: append a legacy event commit to the same ref.
    let refname = format!("refs/forum/threads/{id}");
    let parent_oid = git.resolve_ref(&refname).unwrap().unwrap();
    let tail = Event {
        thread_id: id.into(),
        event_type: EventType::Say,
        created_at: now + chrono::Duration::minutes(1),
        actor: "human/bob".into(),
        body: Some("after-snapshot comment".into()),
        node_type: Some(git_forum::internal::legacy::event::NodeType::Comment),
        ..Event::default()
    };
    let blob = git
        .hash_object(serde_json::to_string(&tail).unwrap().as_bytes())
        .unwrap();
    let tree = git.mktree_single("event.json", &blob).unwrap();
    let tail_commit = git
        .commit_tree(&tree, &[&parent_oid], "tail event")
        .unwrap();
    git.update_ref_cas(&refname, &tail_commit, &parent_oid)
        .unwrap();

    // The tip is now an event commit → read_snapshot returns
    // LegacyEventChain → migration must handle it.
    let pre = snapshot::read_snapshot(&git, id).unwrap_err();
    assert!(matches!(
        pre,
        git_forum::internal::error::ForumError::LegacyEventChain
    ));

    // Direct projection check: strict path must NOT fail.
    let projection = migrate::migrate_legacy_to_snapshot_strict_at(&git, id, &tail_commit).unwrap();
    // Snapshot's pre-existing tags survive the seeding.
    assert!(
        projection
            .doc
            .snapshot
            .tags
            .iter()
            .any(|t| t == "pre-existing"),
        "snapshot-bottom tags must seed the projection, got {:?}",
        projection.doc.snapshot.tags
    );
    // Tail event's comment should appear as a node.
    assert!(
        projection
            .doc
            .nodes
            .iter()
            .any(|n| n.body == "after-snapshot comment"),
        "tail event must be applied to the seeded state"
    );
}

#[test]
fn migrate_cli_handles_mixed_chain_through_full_handler() {
    // Task `9635buy0` follow-up to objection `bf678561`
    // (codex re-check `a985cc17`): the prior fix only exercised
    // `migrate_legacy_to_snapshot_strict_at` directly. The full CLI
    // path (`commands::migrate::run` → `process_one`) used to call
    // `event::load_thread_events_at` BEFORE projection, which parses
    // every ancestor as event.json — so a mixed-chain ref would
    // error on the snapshot ancestor before strict replay ever ran.
    //
    // After the fix, `process_one` walks via `load_event_tail_at`
    // (stops at the snapshot ancestor) and records the ancestor as
    // an `archive`-kind omission so the report explains why
    // `legacy/events.ndjson` carries only the post-snapshot tail.
    use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
    use git_forum::internal::snapshot::{NodeWithBody, ThreadDocument};
    use git_forum::internal::thread::ThreadSnapshot;

    let (repo, git, _paths) = setup();
    let id = "mixchaincli";
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

    // Bottom: a SPEC-3.0 snapshot.
    let mut doc = ThreadDocument::new(ThreadSnapshot {
        schema_version: ThreadSnapshot::SCHEMA_VERSION,
        id: id.into(),
        title: "Mixed CLI".into(),
        category: "rfc".into(),
        status: "draft".into(),
        tags: vec!["seeded".into()],
        created_at: now,
        created_by: "human/alice".into(),
        updated_at: now,
        updated_by: "human/alice".into(),
        branch: None,
        supersedes: vec![],
    });
    doc.nodes.push(NodeWithBody {
        record: NodeRecord {
            id: "seedcomment".into(),
            kind: NodeKind::Comment,
            status: NodeStatus::Open,
            created_at: now,
            created_by: "human/alice".into(),
            updated_at: None,
            updated_by: None,
            reply_to: None,
            legacy_label: None,
        },
        body: "seeded".into(),
    });
    snapshot::write_snapshot(&git, id, &doc, "create snapshot").unwrap();
    let refname = format!("refs/forum/threads/{id}");
    let snapshot_oid = git.resolve_ref(&refname).unwrap().unwrap();

    // Tail: append a legacy event commit so the ref becomes
    // event-tip + snapshot-ancestor.
    let tail = Event {
        thread_id: id.into(),
        event_type: EventType::Say,
        created_at: now + chrono::Duration::minutes(1),
        actor: "human/bob".into(),
        body: Some("after-snapshot tail".into()),
        node_type: Some(git_forum::internal::legacy::event::NodeType::Comment),
        ..Event::default()
    };
    let blob = git
        .hash_object(serde_json::to_string(&tail).unwrap().as_bytes())
        .unwrap();
    let tree = git.mktree_single("event.json", &blob).unwrap();
    let tail_commit = git
        .commit_tree(&tree, &[&snapshot_oid], "tail event")
        .unwrap();
    git.update_ref_cas(&refname, &tail_commit, &snapshot_oid)
        .unwrap();

    // Drive the actual CLI binary.
    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .expect("git-forum migrate should run");
    assert!(
        out.status.success(),
        "migrate exited non-zero on mixed-chain ref:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[migrated]"),
        "mixed-chain ref must be migrated, not skipped/errored:\n{stdout}"
    );

    // Post-migration: read_snapshot must succeed and carry both the
    // seeded state AND the tail event's node.
    let post = snapshot::read_snapshot(&git, id)
        .expect("post-migrate read_snapshot must return a 3.0 snapshot");
    assert!(
        post.snapshot.tags.iter().any(|t| t == "seeded"),
        "snapshot-bottom tag `seeded` must survive, got {:?}",
        post.snapshot.tags
    );
    assert!(
        post.nodes.iter().any(|n| n.body == "after-snapshot tail"),
        "tail event must be incorporated into the migrated snapshot"
    );

    // Report must record the snapshot ancestor as an `archive`-kind
    // omission, so the operator sees why the new
    // legacy/events.ndjson carries only the tail.
    let report_path = repo.path().join(".git/forum/migration-report.json");
    let report_json = std::fs::read_to_string(&report_path)
        .expect("migration-report.json should be written by run()");
    let report: serde_json::Value = serde_json::from_str(&report_json).unwrap();
    let entry = report["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["thread_id"] == id)
        .expect("report should contain the mixed-chain thread");
    let omissions = entry["omissions"].as_array().expect("omissions array");
    let archive_omission = omissions
        .iter()
        .find(|o| o["kind"] == "archive")
        .expect("mixed-chain migration must emit an `archive` omission");
    assert_eq!(
        archive_omission["item"].as_str().unwrap(),
        snapshot_oid,
        "archive omission must point at the snapshot ancestor OID"
    );
}
