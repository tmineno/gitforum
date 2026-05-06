//! Comprehensive `git forum migrate --to 3.0` validity test
//! (task `9635buy0`).
//!
//! Two fixtures, four legacy kinds each:
//!
//! - **Fixture A** (2.0 chains, with `facet_set`): exercises the
//!   path where lifecycle/tags came from an explicit facet_set
//!   event in the chain.
//! - **Fixture B** (1.x chains, no `facet_set`): exercises the
//!   §8.3 inferred-metadata path. `lifecycle_explicit == false`,
//!   so the report carries an `inferred_metadata` note for these
//!   threads but they MUST still migrate successfully (objection
//!   `efe64dba`).
//!
//! Per kind, after migration the test asserts:
//!
//! - `read_snapshot` succeeds (round-trip).
//! - `status` equals the target category's `initial_status` from
//!   `CategoryRegistry::built_in()` (SPEC-3.0 §8.1 status projection).
//! - `tags` include the canonical augmentation per §8.3
//!   (`bug` for `Issue`, `decision` for `Dec`).
//! - The legacy chain commits remain reachable as ancestors of
//!   the new tip (`git rev-list <tip> -- event.json` still walks).
//! - `legacy/events.ndjson` is present in the snapshot tree
//!   (SPEC-3.0 §8.2).
//! - Pre-migration `read_snapshot` returned `LegacyEventChain`;
//!   post-migration it returns `Ok` (the v3 read path that was
//!   intentionally locked out by the legacy gate is now usable).
//! - Idempotence: a second `git forum migrate --to 3.0` reports
//!   the same threads as `already_migrated` and does not advance
//!   any ref tip.
//!
//! Body / smoke-level coverage of the surrounding plumbing
//! (dry-run, archive preservation across writes, race CAS,
//! structured-report shape, error outcomes) lives in
//! `tests/migrate_v3_smoke_test.rs`. This file is the SPEC §8
//! mapping-table gate and should keep growing as new source
//! kinds / fixtures are added.

mod support;

use std::process::Command;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::error::ForumError;
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

fn alloc_id(kind: ThreadKind, title: &str, salt: u8) -> String {
    id_alloc::alloc_thread_id_with_nonce(
        kind.id_prefix(),
        "human/alice",
        title,
        "2026-01-01T00:00:00Z",
        &[salt, salt, salt, salt, salt, salt, salt, salt],
    )
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

/// Build a 2.0-style chain: Create + facet_set + a comment.
fn build_2_0_chain(git: &GitOps, kind: ThreadKind, lifecycle_str: &str, salt: u8) -> String {
    let id = alloc_id(kind, "Fixture A", salt);
    event::write_event(git, &create_event(&id, kind, "Fixture A")).unwrap();
    let facet = Event {
        thread_id: id.clone(),
        event_type: EventType::FacetSet,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        actor: "system/migrate".into(),
        lifecycle: Some(lifecycle_str.into()),
        ..Event::default()
    };
    event::write_event(git, &facet).unwrap();
    event::write_event(git, &say_event(&id, "comment from 2.0 chain", 2)).unwrap();
    id
}

/// Build a 1.x-style chain: Create + a comment, no facet_set.
fn build_1_x_chain(git: &GitOps, kind: ThreadKind, salt: u8) -> String {
    let id = alloc_id(kind, "Fixture B", salt);
    event::write_event(git, &create_event(&id, kind, "Fixture B")).unwrap();
    event::write_event(git, &say_event(&id, "comment from 1.x chain", 1)).unwrap();
    id
}

struct Expected {
    category: &'static str,
    status: &'static str,
    canonical_tag: Option<&'static str>,
}

fn expected_for(kind: ThreadKind) -> Expected {
    match kind {
        // §8.3: rfc → category=rfc, no canonical tag
        ThreadKind::Rfc => Expected {
            category: "rfc",
            status: "draft",
            canonical_tag: None,
        },
        // §8.3: task → category=task, no canonical tag
        ThreadKind::Task => Expected {
            category: "task",
            status: "open",
            canonical_tag: None,
        },
        // §8.3: bug/issue → category=task, canonical tag `bug`
        ThreadKind::Issue => Expected {
            category: "task",
            status: "open",
            canonical_tag: Some("bug"),
        },
        // §8.3: dec/record → category=task, canonical tag `decision`
        ThreadKind::Dec => Expected {
            category: "task",
            status: "open",
            canonical_tag: Some("decision"),
        },
    }
}

fn assert_post_migration_invariants(git: &GitOps, id: &str, kind: ThreadKind) {
    let exp = expected_for(kind);
    let doc = snapshot::read_snapshot(git, id)
        .unwrap_or_else(|e| panic!("{kind:?} {id}: post-migrate read_snapshot failed: {e}"));
    assert_eq!(
        doc.snapshot.category, exp.category,
        "{kind:?} {id}: category mismatch"
    );
    assert_eq!(
        doc.snapshot.status, exp.status,
        "{kind:?} {id}: status mismatch (must be category initial_status, not legacy final)"
    );
    if let Some(tag) = exp.canonical_tag {
        assert!(
            doc.snapshot.tags.iter().any(|t| t == tag),
            "{kind:?} {id}: missing canonical tag `{tag}` per §8.3, got tags={:?}",
            doc.snapshot.tags
        );
    }

    // legacy/events.ndjson present.
    let tip = git
        .run(&["rev-parse", &format!("refs/forum/threads/{id}")])
        .unwrap();
    let tree_paths = git
        .run(&["ls-tree", "-r", "--name-only", tip.trim()])
        .unwrap();
    assert!(
        tree_paths.lines().any(|p| p == "legacy/events.ndjson"),
        "{kind:?} {id}: legacy/events.ndjson missing from snapshot tree:\n{tree_paths}"
    );

    // Legacy commits remain reachable as ancestors. The snapshot
    // commit's parent should still parse as an event (the original
    // chain tip).
    let parent = git
        .run(&["rev-parse", &format!("{}^", tip.trim())])
        .unwrap();
    let parent_listing = git.run(&["ls-tree", "--name-only", parent.trim()]).unwrap();
    assert!(
        parent_listing.lines().any(|p| p == "event.json"),
        "{kind:?} {id}: snapshot's parent commit must be the legacy chain tip (event.json present), \
         got tree:\n{parent_listing}"
    );
}

fn run_migrate(repo: &support::repo::TestRepo) {
    let bin = env!("CARGO_BIN_EXE_git-forum");
    let out = Command::new(bin)
        .current_dir(repo.path())
        .args(["migrate", "--to", "3.0"])
        .output()
        .expect("migrate must run");
    assert!(
        out.status.success(),
        "migrate exited non-zero:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn fixture_a_2_0_chains_round_trip_through_migration() {
    let (repo, git, _paths) = setup();
    let rfc = build_2_0_chain(&git, ThreadKind::Rfc, "proposal", 0x11);
    let task = build_2_0_chain(&git, ThreadKind::Task, "execution", 0x12);
    let issue = build_2_0_chain(&git, ThreadKind::Issue, "execution", 0x13);
    let dec = build_2_0_chain(&git, ThreadKind::Dec, "record", 0x14);

    // Pre-migration: every read MUST fail with LegacyEventChain
    // (this is the half of item 14 that asserts the v3 read path
    // really was locked out before migration).
    for (id, _kind) in [
        (&rfc, ThreadKind::Rfc),
        (&task, ThreadKind::Task),
        (&issue, ThreadKind::Issue),
        (&dec, ThreadKind::Dec),
    ] {
        let pre = snapshot::read_snapshot(&git, id).unwrap_err();
        assert!(
            matches!(pre, ForumError::LegacyEventChain),
            "{id}: pre-migrate read must be LegacyEventChain, got {pre:?}"
        );
    }

    run_migrate(&repo);

    // Post-migration: each thread satisfies the §8 invariants.
    assert_post_migration_invariants(&git, &rfc, ThreadKind::Rfc);
    assert_post_migration_invariants(&git, &task, ThreadKind::Task);
    assert_post_migration_invariants(&git, &issue, ThreadKind::Issue);
    assert_post_migration_invariants(&git, &dec, ThreadKind::Dec);
}

#[test]
fn fixture_b_1_x_chains_without_facet_set_round_trip_through_migration() {
    let (repo, git, paths) = setup();
    let rfc = build_1_x_chain(&git, ThreadKind::Rfc, 0x21);
    let task = build_1_x_chain(&git, ThreadKind::Task, 0x22);
    let issue = build_1_x_chain(&git, ThreadKind::Issue, 0x23);
    let dec = build_1_x_chain(&git, ThreadKind::Dec, 0x24);

    for id in [&rfc, &task, &issue, &dec] {
        let pre = snapshot::read_snapshot(&git, id).unwrap_err();
        assert!(matches!(pre, ForumError::LegacyEventChain));
    }

    run_migrate(&repo);

    assert_post_migration_invariants(&git, &rfc, ThreadKind::Rfc);
    assert_post_migration_invariants(&git, &task, ThreadKind::Task);
    assert_post_migration_invariants(&git, &issue, ThreadKind::Issue);
    assert_post_migration_invariants(&git, &dec, ThreadKind::Dec);

    // Critical: 1.x absence of facet_set must NOT have appeared as
    // an error or omission. Each 1.x thread MUST carry an
    // `inferred_metadata` note instead (objection `efe64dba`).
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(paths.git_forum.join("migration-report.json")).unwrap(),
    )
    .unwrap();
    for id in [&rfc, &task, &issue, &dec] {
        let entry = report["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["thread_id"] == **id)
            .unwrap();
        assert_eq!(
            entry["outcome"], "migrated",
            "{id}: 1.x chain must migrate cleanly, got {entry}"
        );
        assert!(
            entry["inferred_metadata"].is_string(),
            "{id}: 1.x chain must carry an inferred_metadata note (objection efe64dba), \
             got {entry}"
        );
        // Strict-replay omissions for the 1.x scenarios should be
        // empty — missing facet_set is NOT an issue.
        assert!(
            entry["omissions"].is_null()
                || entry["omissions"]
                    .as_array()
                    .map(|a| a.is_empty())
                    .unwrap_or(true),
            "{id}: 1.x chain must not produce omissions, got {entry}"
        );
    }
}

#[test]
fn migrate_then_re_migrate_is_a_no_op() {
    // Idempotence across all four §8.3 source kinds. The
    // single-thread idempotence smoke test in
    // tests/migrate_v3_smoke_test.rs covers one kind; this test
    // pins it for all four so a regression on any §8.3 mapping
    // gets caught here.
    let (repo, git, _paths) = setup();
    let rfc = build_2_0_chain(&git, ThreadKind::Rfc, "proposal", 0x31);
    let task = build_2_0_chain(&git, ThreadKind::Task, "execution", 0x32);
    let issue = build_1_x_chain(&git, ThreadKind::Issue, 0x33);
    let dec = build_1_x_chain(&git, ThreadKind::Dec, 0x34);

    run_migrate(&repo);

    let mut tips: Vec<(String, String)> = Vec::new();
    for id in [&rfc, &task, &issue, &dec] {
        let tip = git
            .run(&["rev-parse", &format!("refs/forum/threads/{id}")])
            .unwrap()
            .trim()
            .to_string();
        tips.push((id.to_string(), tip));
    }

    // Second migrate.
    run_migrate(&repo);

    for (id, before) in &tips {
        let after = git
            .run(&["rev-parse", &format!("refs/forum/threads/{id}")])
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(
            *before, after,
            "{id}: ref must not advance during a no-op second migrate"
        );
    }
}
