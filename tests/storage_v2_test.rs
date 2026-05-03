//! v2.x storage-shape regression tests (task `4w8hm98j`).
//!
//! These tests assert the **event-chain** storage layout at
//! `refs/forum/threads/<id>` produced by the v2 implementation. They
//! are intentionally version-pinned: the matching Phase 2 cutover
//! commit per command removes the corresponding `v2_*` test and adds
//! its `tests/storage_v3_test.rs` counterpart that asserts the
//! snapshot-tree shape.
//!
//! See `doc/internal/cli-coverage-audit.md` for the cutover discipline
//! and `doc/internal/main-rs-audit.md` for per-command Phase 2 slots.

mod support;

use std::process::{Command, Output};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;

fn bin() -> String {
    env!("CARGO_BIN_EXE_git-forum").to_string()
}

fn run_ok(repo: &support::repo::TestRepo, args: &[&str]) -> Output {
    let out = Command::new(bin())
        .current_dir(repo.path())
        .args(args)
        .output()
        .expect("git-forum invocation failed");
    assert!(
        out.status.success(),
        "git-forum {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    out
}

fn extract_created_id(out: &Output) -> String {
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim()
        .strip_prefix("Created ")
        .unwrap_or(s.trim())
        .split_whitespace()
        .next()
        .expect("no thread id in `Created …` line")
        .to_string()
}

fn fresh_repo() -> support::repo::TestRepo {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    repo
}

/// v2 invariant: every commit in `refs/forum/threads/<id>` is an
/// event-chain commit — its tree contains exactly `event.json` and
/// nothing else. Phase 1 replaces this layout with a snapshot tree
/// (`thread.toml` + `nodes/` + ...); the matching v3 test in
/// `tests/storage_v3_test.rs` is unblocked at the `thread_new`
/// cutover commit (slot 1 of Phase 2).
#[test]
fn v2_thread_new_chain_is_event_json_only() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(&repo, &["new", "issue", "v2 shape probe"]));

    let git = GitOps::new(repo.path().to_path_buf());
    let tip_ref = format!("refs/forum/threads/{id}");

    // `git forum new <kind>` emits Create then facet-set, so the chain
    // has at least two commits. Walk every commit and assert each
    // tree is exactly `event.json`.
    let commits = git.run(&["rev-list", &tip_ref]).expect("rev-list tip");
    let shas: Vec<&str> = commits.lines().collect();
    assert!(
        shas.len() >= 2,
        "v2 `new <kind>` should emit at least 2 events; got {} commits",
        shas.len()
    );

    for sha in &shas {
        let tree = git
            .run(&["ls-tree", "-r", "--name-only", sha])
            .expect("ls-tree commit");
        let entries: Vec<&str> = tree.lines().collect();
        assert_eq!(
            entries,
            vec!["event.json"],
            "v2 commit {sha} tree must be exactly [event.json]; got {entries:?}"
        );
    }

    // Tip event.json carries an `event_type` field (snake_case wire format).
    let tip_body = git
        .run(&["cat-file", "-p", &format!("{tip_ref}:event.json")])
        .expect("cat-file tip event.json");
    assert!(
        tip_body.contains("\"event_type\""),
        "v2 event.json missing `event_type` field; body was:\n{tip_body}"
    );
}
