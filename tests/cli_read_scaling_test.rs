//! Commands that read every thread start as many git processes whatever
//! the thread count (`doc/spec/LARGE-FORUM-READS.md` AT-2, AT-3, AT-7).
//!
//! The processes are counted from outside: a `git` wrapper first on `PATH`
//! writes one line per start and runs the real git, so the count does not
//! rely on the code under test.

#![cfg(unix)]

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use git_forum::internal::git_ops::GitOps;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::snapshot::{write_snapshot, NodeWithBody, ThreadDocument};
use git_forum::internal::thread::ThreadSnapshot;
use tempfile::TempDir;

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

/// An initialized forum with `n` threads.
fn forum_of(n: usize) -> support::repo::TestRepo {
    let repo = support::cli::fresh_repo();
    let git = GitOps::new(repo.path().to_path_buf());
    for i in 0..n {
        let id = format!("scale{i:03}");
        write_snapshot(&git, &id, &thread(&id), "create").unwrap();
    }
    repo
}

/// The git found on `PATH`, which the wrapper runs.
fn real_git() -> PathBuf {
    let path = std::env::var_os("PATH").expect("PATH");
    std::env::split_paths(&path)
        .map(|dir| dir.join("git"))
        .find(|git| git.is_file())
        .expect("git on PATH")
}

/// A directory holding a `git` wrapper that appends a line to `log` per
/// start.
fn counting_git(log: &Path) -> TempDir {
    let dir = TempDir::new().unwrap();
    let wrapper = dir.path().join("git");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\necho x >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real_git().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

/// Run `git-forum <args>` in `dir` with the counting wrapper first on
/// `PATH`. Returns the output and the git processes it started.
fn run_counted(dir: &Path, args: &[&str]) -> (Output, usize) {
    let scratch = TempDir::new().unwrap();
    let log = scratch.path().join("git-starts.log");
    let wrapper = counting_git(&log);
    let path = std::env::join_paths(
        std::iter::once(wrapper.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let out = Command::new(support::cli::bin())
        .current_dir(dir)
        .args(args)
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git-forum {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let starts = std::fs::read_to_string(&log).unwrap_or_default();
    (out, starts.lines().count())
}

/// AT-2: `ls` and `shortlog` start as many git processes for 40 threads as
/// for 10.
#[test]
fn ls_and_shortlog_start_as_many_git_processes_for_40_threads_as_for_10() {
    let small = forum_of(10);
    let large = forum_of(40);
    for args in [&["ls"][..], &["shortlog", "--since", "2026-01-01"][..]] {
        let (_, starts_10) = run_counted(small.path(), args);
        let (out_40, starts_40) = run_counted(large.path(), args);
        assert_eq!(starts_10, starts_40, "git-forum {args:?}");
        if args == ["ls"] {
            let listed = String::from_utf8_lossy(&out_40.stdout);
            assert!(listed.contains("Thread scale039"), "{listed}");
        }
    }
}

/// AT-3: `node show` starts as many git processes for 40 threads as for 10.
#[test]
fn node_show_starts_as_many_git_processes_for_40_threads_as_for_10() {
    let small = forum_of(10);
    let large = forum_of(40);
    let (out_10, starts_10) = run_counted(small.path(), &["node", "show", "scale005n1"]);
    let (out_40, starts_40) = run_counted(large.path(), &["node", "show", "scale005n1"]);
    // The history table carries each repository's commit dates and ids, so
    // the outputs are compared on the node itself.
    for out in [&out_10, &out_40] {
        assert!(String::from_utf8_lossy(&out.stdout).contains("Node 1 of scale005"));
    }
    assert_eq!(starts_10, starts_40);
}

/// AT-7: `ls` from a subdirectory of the repository prints what it prints
/// at the root.
#[test]
fn ls_from_a_subdirectory_matches_the_root() {
    let repo = forum_of(3);
    let sub = repo.path().join("nested/deep");
    std::fs::create_dir_all(&sub).unwrap();
    let root = support::cli::run_ok(repo.path(), &["ls"]);
    let nested = support::cli::run_ok(&sub, &["ls"]);
    assert_eq!(nested.stdout, root.stdout);
    assert!(String::from_utf8_lossy(&root.stdout).contains("Thread scale002"));
}
