mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;

fn git_output(repo: &support::repo::TestRepo, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(repo.path())
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("failed to run git")
}

fn git(repo: &support::repo::TestRepo, args: &[&str]) {
    let output = git_output(repo, args);
    assert!(
        output.status.success(),
        "git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Create a corrupt thread ref whose commit has an empty tree (no event.json).
fn create_corrupt_thread_ref(repo: &support::repo::TestRepo, thread_id: &str) {
    // Create an empty tree
    let output = git_output(repo, &["mktree", "--missing"]);
    assert!(output.status.success());
    let empty_tree = String::from_utf8(output.stdout).unwrap().trim().to_string();

    // Create a commit using the empty tree
    let output = git_output(repo, &["commit-tree", &empty_tree, "-m", "dummy event"]);
    assert!(output.status.success());
    let commit_sha = String::from_utf8(output.stdout).unwrap().trim().to_string();

    // Point a thread ref at this corrupt commit
    let ref_name = format!("refs/forum/threads/{thread_id}");
    git(repo, &["update-ref", &ref_name, &commit_sha]);
}

#[test]
fn ls_skips_corrupt_thread_with_warning() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    // Create a valid issue
    let create = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "Valid issue"])
        .output()
        .expect("failed to create issue");
    assert!(create.status.success(), "issue creation failed");

    // Create a corrupt thread ref
    create_corrupt_thread_ref(&repo, "ASK-corrupt1");

    // Run `git forum ls` — should succeed despite the corrupt thread
    let ls = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["ls"])
        .output()
        .expect("failed to run ls");

    let stdout = String::from_utf8(ls.stdout).unwrap();
    let stderr = String::from_utf8(ls.stderr).unwrap();

    // Command should succeed (exit 0)
    assert!(
        ls.status.success(),
        "ls should succeed even with corrupt threads. stderr: {stderr}"
    );

    // The valid issue should appear in the output
    assert!(
        stdout.contains("Valid issue"),
        "valid thread should be listed"
    );

    // The corrupt thread should NOT appear in stdout
    assert!(
        !stdout.contains("ASK-corrupt1"),
        "corrupt thread should not be listed"
    );

    // stderr should warn about the broken thread
    assert!(
        stderr.contains("ASK-corrupt1"),
        "warning should identify the broken thread: {stderr}"
    );
    assert!(
        stderr.contains("warning"),
        "should emit a warning: {stderr}"
    );
    assert!(
        stderr.contains("doctor"),
        "warning should suggest running doctor: {stderr}"
    );
}

#[test]
fn ls_kind_filter_works_with_corrupt_thread() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    // Create a valid RFC
    let create = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["rfc", "new", "Valid RFC", "--body", "## Goal\nTest."])
        .output()
        .expect("failed to create rfc");
    assert!(create.status.success(), "rfc creation failed");

    // Create a corrupt thread ref (pretending to be an RFC)
    create_corrupt_thread_ref(&repo, "RFC-corrupt2");

    // Run `git forum ls --kind rfc` — should succeed
    let ls = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["ls", "--kind", "rfc"])
        .output()
        .expect("failed to run ls --kind rfc");

    let stdout = String::from_utf8(ls.stdout).unwrap();
    let stderr = String::from_utf8(ls.stderr).unwrap();

    assert!(
        ls.status.success(),
        "ls --kind rfc should succeed. stderr: {stderr}"
    );
    assert!(stdout.contains("Valid RFC"), "valid RFC should be listed");
    assert!(
        stderr.contains("RFC-corrupt2"),
        "warning should identify the broken RFC thread: {stderr}"
    );
}
