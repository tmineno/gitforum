mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;

#[test]
fn ls_warns_when_git_forum_is_not_initialized() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
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
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
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

    // Strip GIT_* env vars (GIT_DIR/GIT_WORK_TREE/GIT_INDEX_FILE) inherited
    // from a hook context — otherwise `commit-tree` / `update-ref` can target
    // the wrong index/dir and silently produce garbage refs. Assert exit
    // status on every step so any failure surfaces here, not as a confusing
    // assertion downstream.
    let run_git = |args: &[&str]| -> std::process::Output {
        Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"))
    };
    let stdout_of = |out: &std::process::Output, args: &[&str]| -> String {
        assert!(
            out.status.success(),
            "git {args:?} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout.clone()).unwrap().trim().into()
    };

    let args = &["hash-object", "-t", "tree", "/dev/null"];
    let tree = stdout_of(&run_git(args), args);

    let args = &["commit-tree", &tree, "-m", "dummy event"];
    let commit = stdout_of(&run_git(args), args);

    let args = &["update-ref", "refs/forum/threads/ASK-0001", &commit];
    let update = run_git(args);
    assert!(update.status.success(), "update-ref failed");

    // ls should NOT warn — forum refs exist.
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .arg("ls")
        .output()
        .expect("failed to run git-forum ls");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        !stderr.contains("warning: git-forum is not initialized"),
        "should suppress warning when forum refs exist; stderr: {stderr}"
    );
}
