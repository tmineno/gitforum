//! v3.0 storage-shape regression tests (task `4w8hm98j`).
//!
//! Per-command tests turn on as their Phase 2 cutover lands (see
//! `doc/internal/main-rs-audit.md` slot order). Until the snapshot
//! writer (RFC `7ymtc4b2` Phase 1) is in place, every test here is
//! `#[ignore]`-gated so `cargo test` stays green; the matching
//! Phase 2 cutover commit per command removes the `#[ignore]` and the
//! corresponding entry in `tests/storage_v2_test.rs`.
//!
//! See `doc/internal/cli-coverage-audit.md` for the cutover discipline.

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
    assert!(out.status.success());
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

/// v3 invariant (SPEC-3.0 §4): `git forum new` writes a snapshot tree
/// containing `thread.toml` (and `links.toml` if links were specified)
/// at `refs/forum/threads/<id>`. Unblocks at Phase 2 slot 1
/// (`thread_new` cutover); the v2 counterpart in
/// `tests/storage_v2_test.rs` is removed in the same commit.
#[test]
#[ignore = "unblocked at Phase 2 slot 1 (thread_new cutover) per RFC 7ymtc4b2"]
fn v3_thread_new_writes_thread_toml() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(&repo, &["new", "issue", "v3 shape probe"]));

    let git = GitOps::new(repo.path().to_path_buf());
    let tip_ref = format!("refs/forum/threads/{id}");
    let tip = git.run(&["rev-parse", &tip_ref]).expect("rev-parse tip");

    let tree = git
        .run(&["ls-tree", "-r", "--name-only", tip.trim()])
        .expect("ls-tree tip");
    let entries: Vec<&str> = tree.lines().collect();
    assert!(
        entries.contains(&"thread.toml"),
        "v3 snapshot tree must contain thread.toml; got {entries:?}"
    );

    let body = git
        .run(&["cat-file", "-p", &format!("{}:thread.toml", tip.trim())])
        .expect("cat-file thread.toml");
    assert!(
        body.contains("[thread]"),
        "v3 thread.toml must contain [thread] section; body was:\n{body}"
    );
}
