mod support;

use std::io::Write;
use std::process::{Command, Stdio};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;

#[test]
fn thread_new_accepts_body_from_stdin() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "Parser fails", "--body", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run git-forum issue new");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"Long body from stdin\nwith another line\n")
        .expect("failed to write stdin");

    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(output.status.success());

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, "ISSUE-0001").unwrap();
    assert_eq!(
        state.body.as_deref(),
        Some("Long body from stdin\nwith another line\n")
    );
}

#[test]
fn thread_new_can_create_link_immediately() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let create_rfc = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["rfc", "new", "Switch backend"])
        .output()
        .expect("failed to create rfc");
    assert!(create_rfc.status.success());

    let create_issue = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "issue",
            "new",
            "Implement backend",
            "--link-to",
            "RFC-0001",
            "--rel",
            "implements",
        ])
        .output()
        .expect("failed to create issue with link");
    assert!(create_issue.status.success());

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, "ISSUE-0001").unwrap();
    assert_eq!(state.links.len(), 1);
    assert_eq!(state.links[0].target_thread_id, "RFC-0001");
    assert_eq!(state.links[0].rel, "implements");
}
