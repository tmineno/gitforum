//! `internal::git_ops::GitOps` reads through the batch process
//! (`doc/spec/LARGE-FORUM-READS.md` AT-1 and AT-6, ADR-015).
//!
//! Every read is compared with what one-shot `git cat-file -p` and
//! `git ls-tree -r --full-tree --name-only` give for the same object, so
//! the batch process cannot change a result.

mod support;

use std::path::Path;
use std::process::Command;

use git_forum::internal::evidence::{EvidenceFile, EvidenceKind, EvidenceRecord};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::snapshot::{
    read_snapshot, write_snapshot, Link, Links, NodeWithBody, ThreadDocument,
};
use git_forum::internal::thread::ThreadSnapshot;

/// One-shot git in `repo`, raw stdout; panics on failure. The variables a
/// git hook sets (this suite also runs from pre-commit) are removed so the
/// command reads `repo`.
fn git_raw(repo: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    out.stdout
}

fn node(id: &str, kind: NodeKind, reply_to: Option<&str>, body: &str) -> NodeWithBody {
    NodeWithBody {
        record: NodeRecord {
            id: id.into(),
            kind,
            status: NodeStatus::Open,
            created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
            created_by: "human/alice".into(),
            updated_at: None,
            updated_by: None,
            reply_to: reply_to.map(String::from),
            legacy_label: None,
        },
        body: body.into(),
    }
}

/// A snapshot with every kind of file: a body that ends in blank lines and
/// spaces, CJK and emoji, a node with a body and one without, links and
/// evidence.
fn every_file_kind(id: &str) -> ThreadDocument {
    ThreadDocument {
        snapshot: ThreadSnapshot {
            schema_version: 3,
            id: id.into(),
            title: "キャッシュ方針 🚀".into(),
            category: "rfc".into(),
            status: "draft".into(),
            tags: vec!["bug".into()],
            created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
            created_by: "human/alice".into(),
            updated_at: "2026-05-03T00:00:00Z".parse().unwrap(),
            updated_by: "human/alice".into(),
            branch: None,
            supersedes: vec![],
            visibility: Default::default(),
        },
        body: Some("本文の 1 行目 🚀\n\n| a | b |\n|---|---|\n\n  \n".into()),
        nodes: vec![
            node("node1", NodeKind::Comment, None, "コメント\ntrailing  \n\n"),
            node("node2", NodeKind::Objection, Some("node1"), ""),
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

/// AT-1: `list_tree_files`, `show_file` and `show_file_bytes` give what
/// one-shot `ls-tree` and `cat-file -p` give, for every file of a snapshot
/// with every kind of file.
#[test]
fn batch_reads_match_one_shot_git() {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let original = every_file_kind("BATCH1");
    write_snapshot(&git, "BATCH1", &original, "create").unwrap();
    let tip = git
        .resolve_ref("refs/forum/threads/BATCH1")
        .unwrap()
        .unwrap();

    let listing = git_raw(
        repo.path(),
        &["ls-tree", "-r", "--full-tree", "--name-only", &tip],
    );
    let expected: Vec<String> = String::from_utf8(listing)
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    let files = git.list_tree_files(&tip).unwrap();
    assert_eq!(files, expected);
    for name in ["thread.toml", "body.md", "links.toml", "evidence.toml"] {
        assert!(files.iter().any(|f| f == name), "{name} missing: {files:?}");
    }
    assert!(files.iter().any(|f| f.starts_with("nodes/")), "{files:?}");

    for path in &files {
        let raw = git_raw(repo.path(), &["cat-file", "-p", &format!("{tip}:{path}")]);
        assert_eq!(git.show_file_bytes(&tip, path).unwrap(), raw, "{path}");
        let trimmed = String::from_utf8_lossy(&raw).trim_end().to_string();
        assert_eq!(git.show_file(&tip, path).unwrap(), trimmed, "{path}");
    }

    assert_eq!(read_snapshot(&git, "BATCH1").unwrap(), original);
}

/// The reads above start one process for all of them (the batch process),
/// besides the refs lookups.
#[test]
fn batch_reads_start_one_process() {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    write_snapshot(&git, "BATCH2", &every_file_kind("BATCH2"), "create").unwrap();
    let tip = git
        .resolve_ref("refs/forum/threads/BATCH2")
        .unwrap()
        .unwrap();

    let before = git.spawned_processes();
    let files = git.list_tree_files(&tip).unwrap();
    for path in &files {
        git.show_file_bytes(&tip, path).unwrap();
        git.show_file(&tip, path).unwrap();
    }
    assert_eq!(git.spawned_processes() - before, 1, "{files:?}");
}

/// A path that is not in the tree fails with the error one-shot
/// `cat-file -p` gives, and a directory reads as `cat-file -p` prints it.
#[test]
fn reads_the_batch_cannot_answer_go_the_old_way() {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    write_snapshot(&git, "BATCH3", &every_file_kind("BATCH3"), "create").unwrap();
    let tip = git
        .resolve_ref("refs/forum/threads/BATCH3")
        .unwrap()
        .unwrap();

    let old = git
        .run(&["cat-file", "-p", &format!("{tip}:no-such-file")])
        .unwrap_err()
        .to_string();
    let err = git.show_file(&tip, "no-such-file").unwrap_err().to_string();
    assert_eq!(err, old);
    assert!(git.show_file_bytes(&tip, "no-such-file").is_err());

    let tree = git
        .run(&["cat-file", "-p", &format!("{tip}:nodes")])
        .unwrap();
    assert_eq!(git.show_file(&tip, "nodes").unwrap(), tree);
}

/// AT-6: objects written after the batch process started are read back
/// through the same `GitOps`.
#[test]
fn reads_see_writes_made_after_the_batch_process_started() {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let mut doc = every_file_kind("BATCH4");
    write_snapshot(&git, "BATCH4", &doc, "create").unwrap();
    assert_eq!(read_snapshot(&git, "BATCH4").unwrap().nodes.len(), 2);

    doc.nodes
        .push(node("node3", NodeKind::Action, None, "書いた後に読む\n"));
    write_snapshot(&git, "BATCH4", &doc, "append").unwrap();
    let read = read_snapshot(&git, "BATCH4").unwrap();
    assert_eq!(read, doc);
    assert_eq!(read.nodes[2].body, "書いた後に読む\n");
}
