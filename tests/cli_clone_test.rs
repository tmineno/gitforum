mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;

fn isolation_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ]
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .envs(isolation_env())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git command failed");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn git_forum_cmd(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .args(args)
        .current_dir(dir)
        .envs(isolation_env())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git-forum command failed")
}

#[test]
fn init_adds_refspec_for_existing_remote() {
    let repo = support::repo::TestRepo::new();

    // Add a dummy remote
    git(repo.path(), &["remote", "add", "origin", "https://example.com/repo.git"]);

    // Run init
    let output = git_forum_cmd(repo.path(), &["init"]);
    assert!(output.status.success(), "init failed: {}", String::from_utf8_lossy(&output.stderr));

    // Check refspec was added
    let refspecs = git(repo.path(), &["config", "--get-all", "remote.origin.fetch"]);
    assert!(
        refspecs.contains("+refs/forum/*:refs/forum/*"),
        "refspec not found in: {refspecs}"
    );
}

#[test]
fn init_refspec_is_idempotent() {
    let repo = support::repo::TestRepo::new();

    git(repo.path(), &["remote", "add", "origin", "https://example.com/repo.git"]);

    // Run init twice
    git_forum_cmd(repo.path(), &["init"]);
    git_forum_cmd(repo.path(), &["init"]);

    // Refspec should appear exactly once
    let refspecs = git(repo.path(), &["config", "--get-all", "remote.origin.fetch"]);
    let count = refspecs
        .lines()
        .filter(|l| l.trim() == "+refs/forum/*:refs/forum/*")
        .count();
    assert_eq!(count, 1, "refspec should appear exactly once, got {count} in:\n{refspecs}");
}

#[test]
fn init_fetches_forum_refs_from_remote() {
    // Create an "upstream" repo with forum data
    let upstream = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(upstream.path());
    init::init_forum(&paths).unwrap();

    // Create a seed commit so clone works
    git(upstream.path(), &["add", ".forum"]);
    git(upstream.path(), &["commit", "-m", "seed"]);

    // Create a forum thread ref in the upstream repo
    let tree = git(upstream.path(), &["hash-object", "-t", "tree", "/dev/null"]);
    let commit = git(upstream.path(), &["commit-tree", &tree, "-m", "forum event"]);
    git(
        upstream.path(),
        &["update-ref", "refs/forum/threads/ASK-0001", &commit],
    );

    // Clone with --no-hardlinks (simulates non-local clone)
    let clone_dir = tempfile::TempDir::new().unwrap();
    let clone_path = clone_dir.path().join("cloned");
    let status = Command::new("git")
        .args([
            "clone",
            "--no-hardlinks",
            &upstream.path().to_string_lossy(),
            &clone_path.to_string_lossy(),
        ])
        .envs(isolation_env())
        .output()
        .expect("git clone failed");
    assert!(status.status.success(), "clone failed");

    // Verify forum refs are NOT present before init
    let refs_before = git(&clone_path, &["for-each-ref", "refs/forum/"]);
    assert!(
        refs_before.is_empty(),
        "forum refs should not be present before init: {refs_before}"
    );

    // Run git forum init in the clone
    let output = git_forum_cmd(&clone_path, &["init"]);
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify forum refs ARE present after init
    let refs_after = git(&clone_path, &["for-each-ref", "refs/forum/"]);
    assert!(
        refs_after.contains("refs/forum/threads/ASK-0001"),
        "forum refs should be present after init: {refs_after}"
    );
}

#[test]
fn doctor_warns_on_missing_refspec() {
    let repo = support::repo::TestRepo::new();

    // Init first (adds refspec)
    git(repo.path(), &["remote", "add", "origin", "https://example.com/repo.git"]);
    git_forum_cmd(repo.path(), &["init"]);

    // Remove the refspec
    Command::new("git")
        .args([
            "config",
            "--unset",
            "remote.origin.fetch",
            r"\+refs/forum/\*:refs/forum/\*",
        ])
        .current_dir(repo.path())
        .envs(isolation_env())
        .output()
        .expect("unset failed");

    // Run doctor
    let output = git_forum_cmd(repo.path(), &["doctor"]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("forum refspec (origin)")
            && (combined.contains("WARN") || combined.contains("warn")),
        "doctor should warn about missing refspec:\n{combined}"
    );
}

#[test]
fn doctor_ok_when_no_remotes() {
    let repo = support::repo::TestRepo::new();
    git_forum_cmd(repo.path(), &["init"]);

    let output = git_forum_cmd(repo.path(), &["doctor", "--verbose"]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("forum refspec (no remotes)"),
        "doctor should report ok for no remotes:\n{combined}"
    );
}
