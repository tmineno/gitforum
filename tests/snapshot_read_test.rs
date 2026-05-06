//! `internal::snapshot::store::read_snapshot` integration tests
//! (RFC `7ymtc4b2`, task `qa8u71j9`).
//!
//! Covers the round-trip with `write_snapshot`, the `LegacyEventChain`
//! pre-flight, the `SnapshotMissing` cases (no ref, no thread.toml),
//! and `SnapshotInvalid` for malformed payload.
//!
//! Direct snapshot-store tests; unrelated to the CLI cutover gates in
//! `tests/storage_v3_test.rs`.

mod support;

use git_forum::internal::error::ForumError;
use git_forum::internal::evidence::{EvidenceFile, EvidenceKind, EvidenceRecord};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::snapshot::{
    read_snapshot, write_snapshot, Link, Links, NodeWithBody, ThreadDocument,
};
use git_forum::internal::thread::ThreadSnapshot;

fn fresh_repo() -> support::repo::TestRepo {
    support::repo::TestRepo::new()
}

fn sample_thread(id: &str) -> ThreadSnapshot {
    ThreadSnapshot {
        schema_version: 3,
        id: id.into(),
        title: "T".into(),
        category: "rfc".into(),
        status: "draft".into(),
        tags: vec!["cross-cutting".into()],
        created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
        created_by: "human/alice".into(),
        updated_at: "2026-05-03T00:00:00Z".parse().unwrap(),
        updated_by: "human/alice".into(),
        branch: None,
        supersedes: vec![],
    }
}

fn full_doc(id: &str) -> ThreadDocument {
    ThreadDocument {
        snapshot: sample_thread(id),
        body: Some("Body text.\n".into()),
        nodes: vec![
            NodeWithBody {
                record: NodeRecord {
                    id: "node1".into(),
                    kind: NodeKind::Comment,
                    status: NodeStatus::Open,
                    created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
                    created_by: "human/alice".into(),
                    updated_at: None,
                    updated_by: None,
                    reply_to: None,
                    legacy_label: None,
                },
                body: "first node body\n".into(),
            },
            NodeWithBody {
                record: NodeRecord {
                    id: "node2".into(),
                    kind: NodeKind::Objection,
                    status: NodeStatus::Open,
                    created_at: "2026-05-03T00:01:00Z".parse().unwrap(),
                    created_by: "ai/codex".into(),
                    updated_at: None,
                    updated_by: None,
                    reply_to: Some("node1".into()),
                    legacy_label: None,
                },
                body: "second node body\n".into(),
            },
        ],
        links: Links {
            entries: vec![Link {
                target: "OTHER".into(),
                rel: "implements".into(),
                created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
                created_by: "human/alice".into(),
            }],
        },
        evidence: EvidenceFile {
            entries: vec![EvidenceRecord {
                id: "ev1".into(),
                kind: EvidenceKind::Commit,
                ref_target: "HEAD".into(),
                created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
                created_by: "human/alice".into(),
            }],
        },
    }
}

#[test]
fn read_after_write_round_trip_minimal() {
    let repo = fresh_repo();
    let git = GitOps::new(repo.path().to_path_buf());

    let original = ThreadDocument::new(sample_thread("MIN001"));
    write_snapshot(&git, "MIN001", &original, "create").unwrap();

    let loaded = read_snapshot(&git, "MIN001").unwrap();
    assert_eq!(loaded, original);
}

#[test]
fn read_works_when_gitops_root_is_subdir_of_repo() {
    // Regression: `read_snapshot_at` invokes `git ls-tree -r
    // --name-only <tip>` (and friends in the legacy/doctor paths).
    // Without `--full-tree`, those listings are filtered by the
    // process working directory relative to the repo root, so
    // constructing `GitOps` with a subdirectory as its root made
    // every read return `SnapshotMissing("commit X lacks
    // thread.toml")` even though the tree contained one.
    let repo = fresh_repo();
    let root_git = GitOps::new(repo.path().to_path_buf());
    let original = full_doc("SUBDIR1");
    write_snapshot(&root_git, "SUBDIR1", &original, "create").unwrap();

    let subdir = repo.path().join("nested/deep");
    std::fs::create_dir_all(&subdir).unwrap();
    let subdir_git = GitOps::new(subdir);
    let loaded = read_snapshot(&subdir_git, "SUBDIR1").unwrap();
    assert_eq!(loaded, original);
}

#[test]
fn read_after_write_round_trip_full() {
    let repo = fresh_repo();
    let git = GitOps::new(repo.path().to_path_buf());

    let original = full_doc("FULL01");
    write_snapshot(&git, "FULL01", &original, "create").unwrap();

    let loaded = read_snapshot(&git, "FULL01").unwrap();
    assert_eq!(loaded, original);
    assert_eq!(loaded.nodes.len(), 2);
    assert_eq!(loaded.nodes[0].record.id, "node1");
    assert_eq!(loaded.nodes[1].record.id, "node2");
}

#[test]
fn read_missing_ref_returns_snapshot_missing() {
    let repo = fresh_repo();
    let git = GitOps::new(repo.path().to_path_buf());

    let err = read_snapshot(&git, "DOES_NOT_EXIST").unwrap_err();
    assert!(
        matches!(err, ForumError::SnapshotMissing(_)),
        "expected SnapshotMissing, got {err:?}"
    );
}

#[test]
fn read_tip_without_thread_toml_returns_snapshot_missing() {
    let repo = fresh_repo();
    let git = GitOps::new(repo.path().to_path_buf());

    // Build a tree with only a foreign file — no thread.toml.
    let blob = git.hash_object(b"hi").unwrap();
    let tree = git.mktree_single("README.md", &blob).unwrap();
    let commit = git.commit_tree(&tree, &[], "no thread.toml").unwrap();
    git.create_ref("refs/forum/threads/NO_TOML", &commit)
        .unwrap();

    let err = read_snapshot(&git, "NO_TOML").unwrap_err();
    assert!(matches!(err, ForumError::SnapshotMissing(_)), "{err:?}");
}

#[test]
fn read_legacy_event_chain_returns_legacy_event_chain() {
    let repo = fresh_repo();
    let git = GitOps::new(repo.path().to_path_buf());

    // Mimic a v2 event tip: tree contains only `event.json`.
    let blob = git.hash_object(br#"{"event_type":"create"}"#).unwrap();
    let tree = git.mktree_single("event.json", &blob).unwrap();
    let commit = git.commit_tree(&tree, &[], "v2 event tip").unwrap();
    git.create_ref("refs/forum/threads/V2EVT", &commit).unwrap();

    let err = read_snapshot(&git, "V2EVT").unwrap_err();
    assert!(matches!(err, ForumError::LegacyEventChain), "{err:?}");
}

#[test]
fn read_absent_schema_version_returns_snapshot_schema_unsupported() {
    // Per SPEC-3.0 §11 SnapshotSchemaUnsupported triggers on either
    // an *absent* or *unsupported* `schema_version`. Codex objection
    // 2890e3edd4983bd3 on qa8u71j9: the absent case must not fall
    // through to a generic TOML missing-field error.
    let repo = fresh_repo();
    let git = GitOps::new(repo.path().to_path_buf());

    let bad_toml = r#"
        id = "NOVER1"
        title = "no schema_version"
        category = "rfc"
        status = "draft"
        tags = []
        created_at = "2026-05-03T00:00:00Z"
        created_by = "human/alice"
        updated_at = "2026-05-03T00:00:00Z"
        updated_by = "human/alice"
    "#;
    let blob = git.hash_object(bad_toml.as_bytes()).unwrap();
    let tree = git.mktree_single("thread.toml", &blob).unwrap();
    let commit = git
        .commit_tree(&tree, &[], "absent schema_version")
        .unwrap();
    git.create_ref("refs/forum/threads/NOVER1", &commit)
        .unwrap();

    let err = read_snapshot(&git, "NOVER1").unwrap_err();
    assert!(
        matches!(err, ForumError::SnapshotSchemaUnsupported(_)),
        "expected SnapshotSchemaUnsupported for absent schema_version, got {err:?}"
    );
}

#[test]
fn read_unsupported_schema_version_is_rejected() {
    let repo = fresh_repo();
    let git = GitOps::new(repo.path().to_path_buf());

    let mut bad = sample_thread("OLD001");
    bad.schema_version = 2;
    let bad_toml = toml::to_string(&bad).unwrap();
    let blob = git.hash_object(bad_toml.as_bytes()).unwrap();
    let tree = git.mktree_single("thread.toml", &blob).unwrap();
    let commit = git.commit_tree(&tree, &[], "v2 schema").unwrap();
    git.create_ref("refs/forum/threads/OLD001", &commit)
        .unwrap();

    let err = read_snapshot(&git, "OLD001").unwrap_err();
    assert!(
        matches!(err, ForumError::SnapshotSchemaUnsupported(_)),
        "{err:?}"
    );
}

#[test]
fn read_malformed_thread_toml_returns_toml_error() {
    let repo = fresh_repo();
    let git = GitOps::new(repo.path().to_path_buf());

    let blob = git.hash_object(b"not = valid = toml").unwrap();
    let tree = git.mktree_single("thread.toml", &blob).unwrap();
    let commit = git.commit_tree(&tree, &[], "garbage thread.toml").unwrap();
    git.create_ref("refs/forum/threads/JUNK01", &commit)
        .unwrap();

    let err = read_snapshot(&git, "JUNK01").unwrap_err();
    assert!(matches!(err, ForumError::Toml(_)), "{err:?}");
}
