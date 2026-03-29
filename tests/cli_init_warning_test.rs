mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;

#[test]
fn ls_warns_when_git_forum_is_not_initialized() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("ls")
        .output()
        .expect("failed to run git-forum ls");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(stderr.contains("warning: git-forum is not initialized"));
}

#[test]
fn ls_does_not_warn_after_init() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("ls")
        .output()
        .expect("failed to run git-forum ls");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(!stderr.contains("warning: git-forum is not initialized"));
}

#[test]
fn ls_does_not_warn_when_forum_refs_exist_without_init() {
    let repo = support::repo::TestRepo::new();

    // Create a dummy commit and point a forum thread ref at it (without calling init).
    let tree_hash = Command::new("git")
        .args(["hash-object", "-t", "tree", "/dev/null"])
        .current_dir(repo.path())
        .output()
        .expect("hash-object failed");
    let tree = String::from_utf8(tree_hash.stdout)
        .unwrap()
        .trim()
        .to_string();

    let commit_hash = Command::new("git")
        .args(["commit-tree", &tree, "-m", "dummy event"])
        .current_dir(repo.path())
        .output()
        .expect("commit-tree failed");
    let commit = String::from_utf8(commit_hash.stdout)
        .unwrap()
        .trim()
        .to_string();

    let update = Command::new("git")
        .args(["update-ref", "refs/forum/threads/ASK-0001", &commit])
        .current_dir(repo.path())
        .output()
        .expect("update-ref failed");
    assert!(update.status.success());

    // ls should NOT warn — forum refs exist.
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("ls")
        .output()
        .expect("failed to run git-forum ls");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        !stderr.contains("warning: git-forum is not initialized"),
        "should suppress warning when forum refs exist; stderr: {stderr}"
    );
}
