//! `internal::snapshot::list` on forums of different sizes
//! (`doc/spec/LARGE-FORUM-READS.md` AT-4, AT-5).

mod support;

use git_forum::internal::git_ops::GitOps;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::snapshot::list::{list_threads, list_threads_reusing, ListCache};
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

/// AT-5: with the cache, a refresh reads only the threads that changed —
/// a node added, a thread created, a thread ref deleted — and lists what
/// `list_threads` lists each time.
#[test]
fn list_threads_reusing_reads_only_the_threads_that_changed() {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    for i in 0..5 {
        let id = format!("SCALE{i:03}");
        write_snapshot(&git, &id, &thread(&id), "create").unwrap();
    }
    let listed = |git: &GitOps| format!("{:?}", list_threads(git).unwrap());
    let mut cache = ListCache::default();

    let (rows, read) = list_threads_reusing(&git, &mut cache).unwrap();
    assert_eq!(read.len(), 5);
    assert_eq!(format!("{rows:?}"), listed(&git));

    let (rows, read) = list_threads_reusing(&git, &mut cache).unwrap();
    assert!(read.is_empty(), "{read:?}");
    assert_eq!(format!("{rows:?}"), listed(&git));

    let mut changed = thread("SCALE002");
    changed.nodes.pop();
    changed.snapshot.status = "closed".into();
    write_snapshot(&git, "SCALE002", &changed, "change").unwrap();
    let (rows, read) = list_threads_reusing(&git, &mut cache).unwrap();
    assert_eq!(read, ["SCALE002"]);
    assert_eq!(format!("{rows:?}"), listed(&git));
    assert!(rows
        .iter()
        .any(|r| r.id == "SCALE002" && r.status == "closed"));

    write_snapshot(&git, "SCALE009", &thread("SCALE009"), "create").unwrap();
    let (rows, read) = list_threads_reusing(&git, &mut cache).unwrap();
    assert_eq!(read, ["SCALE009"]);
    assert_eq!(format!("{rows:?}"), listed(&git));

    git.delete_ref("refs/forum/threads/SCALE001").unwrap();
    let (rows, read) = list_threads_reusing(&git, &mut cache).unwrap();
    assert!(read.is_empty(), "{read:?}");
    assert_eq!(format!("{rows:?}"), listed(&git));
    assert!(rows.iter().all(|r| r.id != "SCALE001"));
}
