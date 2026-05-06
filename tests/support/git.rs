//! Shared raw-git and tree/blob helpers.
//!
//! Centralizes the `git`/`create_real_branch` raw-process helpers
//! used by branch-scope tests, plus the `ls-tree`/`cat-file` lookups
//! used by storage-shape tests in `storage_v3_test.rs` and
//! `snapshot_store_test.rs`. Each `tests/*_test.rs` compiles
//! `support` independently, so unused helpers must not warn.

#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

use git_forum::internal::git_ops::GitOps;

/// Run a raw `git <args>` command in `repo_path` and assert success.
/// `GIT_DIR` / `GIT_WORK_TREE` / `GIT_INDEX_FILE` are scrubbed so the
/// test repo is unambiguously the working tree.
pub fn git(repo_path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Seed a real branch on the test repo: empty initial commit + branch
/// pointing at HEAD. Required by CLI arms (notably `branch bind`) that
/// validate the target ref exists locally.
pub fn create_real_branch(repo_path: &Path, branch: &str) {
    git(repo_path, &["commit", "--allow-empty", "-m", "init"]);
    git(repo_path, &["branch", branch]);
}

/// Sorted `ls-tree -r --name-only` of the tip of `refname`.
///
/// Sorting is harmless for the assertion shapes used by callers
/// (`assert_eq!`, `.iter().any()`, `.iter().all()`) and gives stable
/// output for whole-vector comparisons.
pub fn list_tree_paths(git_ops: &GitOps, refname: &str) -> Vec<String> {
    let tip = git_ops
        .run(&["rev-parse", refname])
        .unwrap_or_else(|e| panic!("rev-parse {refname}: {e}"));
    let out = git_ops
        .run(&["ls-tree", "-r", "--name-only", tip.trim()])
        .unwrap_or_else(|e| panic!("ls-tree {refname}: {e}"));
    let mut paths: Vec<String> = out.lines().map(|s| s.to_string()).collect();
    paths.sort();
    paths
}

/// `cat-file -p <tip>:<path>` for the tip of `refname`.
pub fn read_blob(git_ops: &GitOps, refname: &str, path: &str) -> String {
    let tip = git_ops
        .run(&["rev-parse", refname])
        .unwrap_or_else(|e| panic!("rev-parse {refname}: {e}"));
    git_ops
        .run(&["cat-file", "-p", &format!("{}:{path}", tip.trim())])
        .unwrap_or_else(|e| panic!("cat-file {path} on {refname}: {e}"))
}

/// Convenience: sorted tree listing at `refs/forum/threads/<id>`.
pub fn ls_thread_tip(git_ops: &GitOps, id: &str) -> Vec<String> {
    list_tree_paths(git_ops, &format!("refs/forum/threads/{id}"))
}

/// Convenience: read `path` from the tip of `refs/forum/threads/<id>`.
pub fn read_thread_file(git_ops: &GitOps, id: &str, path: &str) -> String {
    read_blob(git_ops, &format!("refs/forum/threads/{id}"), path)
}
