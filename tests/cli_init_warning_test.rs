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
