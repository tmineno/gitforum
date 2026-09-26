//! `internal::snapshot::list` on forums of different sizes
//! (`doc/spec/LARGE-FORUM-READS.md` AT-4).

mod support;

use git_forum::internal::git_ops::GitOps;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::snapshot::list::list_threads;
use git_forum::internal::snapshot::{write_snapshot, NodeWithBody, ThreadDocument};
use git_forum::internal::thread::ThreadSnapshot;

/// A thread with a body and three nodes.
fn thread(id: &str) -> ThreadDocument {
    let at = "2026-05-03T00:00:00Z".parse().unwrap();
    let mut doc = ThreadDocument::new(ThreadSnapshot {
        schema_version: 3,
        id: id.into(),
        title: format!("Thread {id}"),
        category: "issue".into(),
        status: "open".into(),
        tags: vec![],
        created_at: at,
        created_by: "human/alice".into(),
        updated_at: at,
        updated_by: "human/alice".into(),
        branch: None,
        supersedes: vec![],
        visibility: Default::default(),
    });
    doc.body = Some(format!("Body of {id}.\n"));
    for n in 0..3 {
        doc.nodes.push(NodeWithBody {
            record: NodeRecord {
                id: format!("{id}n{n}"),
                kind: NodeKind::Comment,
                status: NodeStatus::Open,
                created_at: at,
                created_by: "human/alice".into(),
                updated_at: None,
                updated_by: None,
                reply_to: None,
                legacy_label: None,
            },
            body: format!("Node {n} of {id}.\n"),
        });
    }
    doc
}

/// Rows `list_threads` returns on a forum of `n` threads, and the git
/// processes it started.
fn list_forum_of(n: usize) -> (usize, usize) {
    let repo = support::repo::TestRepo::new();
    let writer = GitOps::new(repo.path().to_path_buf());
    for i in 0..n {
        let id = format!("SCALE{i:03}");
        write_snapshot(&writer, &id, &thread(&id), "create").unwrap();
    }
    let git = GitOps::new(repo.path().to_path_buf());
    let rows = list_threads(&git).unwrap();
    (rows.len(), git.spawned_processes())
}

/// AT-4: reading the thread list starts as many git processes for 40
/// threads as for 10.
#[test]
fn list_threads_starts_as_many_git_processes_for_40_threads_as_for_10() {
    let (rows_10, spawned_10) = list_forum_of(10);
    let (rows_40, spawned_40) = list_forum_of(40);
    assert_eq!((rows_10, rows_40), (10, 40));
    assert_eq!(spawned_10, spawned_40);
}
