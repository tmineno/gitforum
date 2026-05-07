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
    git(
        repo.path(),
        &["remote", "add", "origin", "https://example.com/repo.git"],
    );

    // Run init
    let output = git_forum_cmd(repo.path(), &["init"]);
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check refspecs were added. RFC fls856j3 §3 trusted-collaborator
    // mode installs the two narrow refspecs; the legacy wildcard is
    // accepted on already-configured clones but not produced by a
    // fresh init.
    let refspecs = git(repo.path(), &["config", "--get-all", "remote.origin.fetch"]);
    assert!(
        refspecs.contains("+refs/forum/threads/*:refs/forum/threads/*"),
        "threads refspec not found in: {refspecs}"
    );
    assert!(
        refspecs.contains("+refs/forum/published/*:refs/forum/published/*"),
        "published refspec not found in: {refspecs}"
    );
}

#[test]
fn init_refspec_is_idempotent() {
    let repo = support::repo::TestRepo::new();

    git(
        repo.path(),
        &["remote", "add", "origin", "https://example.com/repo.git"],
    );

    // Run init twice
    git_forum_cmd(repo.path(), &["init"]);
    git_forum_cmd(repo.path(), &["init"]);

    // Each forum refspec should appear exactly once (RFC fls856j3 §3).
    let refspecs = git(repo.path(), &["config", "--get-all", "remote.origin.fetch"]);
    for want in [
        "+refs/forum/threads/*:refs/forum/threads/*",
        "+refs/forum/published/*:refs/forum/published/*",
    ] {
        let count = refspecs.lines().filter(|l| l.trim() == want).count();
        assert_eq!(
            count, 1,
            "{want} should appear exactly once, got {count} in:\n{refspecs}"
        );
    }
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
    let commit = git(
        upstream.path(),
        &["commit-tree", &tree, "-m", "forum event"],
    );
    git(
        upstream.path(),
        &["update-ref", "refs/forum/threads/ASK-0001", &commit],
    );

    // Clone with --no-hardlinks (simulates non-local clone).
    //
    // current_dir + env_remove are critical: when run from a pre-commit hook,
    // `GIT_INDEX_FILE=.git/index` is inherited as a relative path. Without
    // current_dir, git would resolve it relative to the test process's cwd
    // (the worktree being committed) and clone-time index updates would
    // *overwrite the worktree's own index*, corrupting the parent commit.
    let clone_dir = tempfile::TempDir::new().unwrap();
    let clone_path = clone_dir.path().join("cloned");
    let status = Command::new("git")
        .args([
            "clone",
            "--no-hardlinks",
            &upstream.path().to_string_lossy(),
            &clone_path.to_string_lossy(),
        ])
        .current_dir(clone_dir.path())
        .envs(isolation_env())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
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

    // Init first (adds refspec).
    git(
        repo.path(),
        &["remote", "add", "origin", "https://example.com/repo.git"],
    );
    let init_out = git_forum_cmd(repo.path(), &["init"]);
    assert!(
        init_out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );

    // Remove every forum-namespace refspec (legacy wildcard plus the
    // RFC fls856j3 §3 narrow forms). `git()` strips inherited GIT_DIR
    // / GIT_WORK_TREE / GIT_INDEX_FILE so the unset operates on the
    // test repo even from a pre-commit hook context.
    for pattern in [
        r"\+refs/forum/\*:refs/forum/\*",
        r"\+refs/forum/threads/\*:refs/forum/threads/\*",
        r"\+refs/forum/published/\*:refs/forum/published/\*",
    ] {
        git(
            repo.path(),
            &["config", "--unset-all", "remote.origin.fetch", pattern],
        );
    }

    // Verify the unset took effect.
    let remaining = git(repo.path(), &["config", "--get-all", "remote.origin.fetch"]);
    for v in [
        "+refs/forum/*:refs/forum/*",
        "+refs/forum/threads/*:refs/forum/threads/*",
        "+refs/forum/published/*:refs/forum/published/*",
    ] {
        assert!(
            !remaining.lines().any(|l| l.trim() == v),
            "forum refspec '{v}' should be unset before invoking doctor; remaining:\n{remaining}"
        );
    }

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

#[test]
fn doctor_detects_fresh_clone_signature() {
    // Build an upstream repo that has been initialized and seeded with
    // a forum thread ref, then clone it. The clone has `.forum/`
    // tracked in the worktree, no `.git/forum/` yet, and no
    // `refs/forum/*` refs (default clone refspec doesn't fetch them).
    // doctor must now surface this as a single "fresh clone detected"
    // WARN with an init hint, not as a hard FAIL on missing
    // `.git/forum/`.
    let upstream = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(upstream.path());
    init::init_forum(&paths).unwrap();
    git(upstream.path(), &["add", ".forum"]);
    git(upstream.path(), &["commit", "-m", "seed"]);
    let tree = git(upstream.path(), &["hash-object", "-t", "tree", "/dev/null"]);
    let commit = git(
        upstream.path(),
        &["commit-tree", &tree, "-m", "forum event"],
    );
    git(
        upstream.path(),
        &["update-ref", "refs/forum/threads/ASK-0001", &commit],
    );

    let clone_dir = tempfile::TempDir::new().unwrap();
    let clone_path = clone_dir.path().join("cloned");
    let status = Command::new("git")
        .args([
            "clone",
            "--no-hardlinks",
            &upstream.path().to_string_lossy(),
            &clone_path.to_string_lossy(),
        ])
        .current_dir(clone_dir.path())
        .envs(isolation_env())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git clone failed");
    assert!(status.status.success(), "clone failed");

    // Run doctor (NOT init) on the fresh clone.
    let output = git_forum_cmd(&clone_path, &["doctor"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("fresh clone detected") && combined.contains("git forum init"),
        "doctor should surface fresh-clone hint:\n{combined}"
    );
    // Doctor must not exit non-zero on a benign post-clone state — the
    // .git/forum/ FAIL is downgraded to WARN when the fresh-clone
    // signature matches.
    assert!(
        output.status.success(),
        "doctor should not fail on a fresh clone; got status {:?}\n{combined}",
        output.status
    );
}

#[test]
fn init_reports_fetched_thread_count() {
    let upstream = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(upstream.path());
    init::init_forum(&paths).unwrap();
    git(upstream.path(), &["add", ".forum"]);
    git(upstream.path(), &["commit", "-m", "seed"]);
    let tree = git(upstream.path(), &["hash-object", "-t", "tree", "/dev/null"]);
    for id in ["ASK-0001", "ASK-0002", "ASK-0003"] {
        let commit = git(
            upstream.path(),
            &["commit-tree", &tree, "-m", "forum event"],
        );
        git(
            upstream.path(),
            &["update-ref", &format!("refs/forum/threads/{id}"), &commit],
        );
    }

    let clone_dir = tempfile::TempDir::new().unwrap();
    let clone_path = clone_dir.path().join("cloned");
    let status = Command::new("git")
        .args([
            "clone",
            "--no-hardlinks",
            &upstream.path().to_string_lossy(),
            &clone_path.to_string_lossy(),
        ])
        .current_dir(clone_dir.path())
        .envs(isolation_env())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git clone failed");
    assert!(status.status.success(), "clone failed");

    let output = git_forum_cmd(&clone_path, &["init"]);
    assert!(output.status.success(), "init failed");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("Fetched 3 forum thread refs from 'origin'"),
        "init should report fetched thread count:\n{combined}"
    );
    assert!(
        combined.contains("Detected existing forum config"),
        "init should announce existing forum config on a fresh clone:\n{combined}"
    );
}

// --------------------------------------------------------------------
// RFC fls856j3 §3 transport-mode tests.
// --------------------------------------------------------------------

#[test]
fn init_public_only_skips_threads_refspec() {
    let repo = support::repo::TestRepo::new();
    git(
        repo.path(),
        &["remote", "add", "origin", "https://example.com/repo.git"],
    );

    let out = git_forum_cmd(repo.path(), &["init", "--public-only"]);
    assert!(
        out.status.success(),
        "init --public-only failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let refspecs = git(repo.path(), &["config", "--get-all", "remote.origin.fetch"]);
    assert!(
        refspecs.contains("+refs/forum/published/*:refs/forum/published/*"),
        "published refspec missing: {refspecs}"
    );
    assert!(
        !refspecs.contains("+refs/forum/threads/*:refs/forum/threads/*"),
        "threads refspec must be absent in --public-only mode: {refspecs}"
    );
    assert!(
        !refspecs.contains("+refs/forum/*:refs/forum/*"),
        "wildcard refspec must be absent in --public-only mode: {refspecs}"
    );
}

#[test]
fn init_public_only_strips_legacy_wildcard() {
    // A clone that previously ran the pre-RFC init has the
    // `+refs/forum/*:refs/forum/*` wildcard configured. Switching
    // it to --public-only must remove the wildcard so subsequent
    // fetches don't smuggle authoritative refs across.
    let repo = support::repo::TestRepo::new();
    git(
        repo.path(),
        &["remote", "add", "origin", "https://example.com/repo.git"],
    );
    git(
        repo.path(),
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/forum/*:refs/forum/*",
        ],
    );

    let out = git_forum_cmd(repo.path(), &["init", "--public-only"]);
    assert!(out.status.success());

    let refspecs = git(repo.path(), &["config", "--get-all", "remote.origin.fetch"]);
    assert!(
        !refspecs.contains("+refs/forum/*:refs/forum/*"),
        "legacy wildcard must be removed: {refspecs}"
    );
    assert!(
        refspecs.contains("+refs/forum/published/*:refs/forum/published/*"),
        "published refspec missing: {refspecs}"
    );
}

#[test]
fn init_auto_push_configures_published_only_push_refspec() {
    let repo = support::repo::TestRepo::new();
    git(
        repo.path(),
        &["remote", "add", "origin", "https://example.com/repo.git"],
    );

    let out = git_forum_cmd(repo.path(), &["init", "--auto-push"]);
    assert!(
        out.status.success(),
        "init --auto-push failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let push_refspecs = git(repo.path(), &["config", "--get-all", "remote.origin.push"]);
    assert!(
        push_refspecs.contains("+refs/forum/published/*:refs/forum/published/*"),
        "auto-push must install published push refspec: {push_refspecs}"
    );
    assert!(
        !push_refspecs.contains("refs/forum/threads/"),
        "auto-push must never install a threads-namespace push refspec: {push_refspecs}"
    );
}

#[test]
fn legacy_wildcard_is_recognized_as_having_forum_refspec() {
    // Pure unit-style coverage of `has_forum_refspec`: an existing
    // wildcard must continue to count as "configured" so older
    // clones don't get a spurious "Added refspec" announcement on
    // every `init`.
    let repo = support::repo::TestRepo::new();
    let git_ops = git_forum::internal::git_ops::GitOps::new(repo.path().to_path_buf());
    git(
        repo.path(),
        &["remote", "add", "origin", "https://example.com/repo.git"],
    );
    git(
        repo.path(),
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/forum/*:refs/forum/*",
        ],
    );
    assert!(init::has_forum_refspec(&git_ops, "origin").unwrap());
}
