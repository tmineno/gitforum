mod support;

use std::io::Write;
use std::process::{Command, Stdio};

use git_forum::internal::clock::SystemClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::policy::Policy;
use git_forum::internal::state_change;
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
    let state = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(
        state.body.as_deref(),
        Some("Long body from stdin\nwith another line\n")
    );
}

#[test]
fn thread_new_body_stdin_rejects_empty_input() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "Empty body", "--body", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run git-forum issue new");

    // Close stdin immediately without writing anything
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(
        !output.status.success(),
        "empty stdin should cause failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("empty input"),
        "error should mention empty input: {stderr}"
    );
}

#[test]
fn thread_new_body_stdin_rejects_whitespace_only() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "Whitespace body", "--body", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run git-forum issue new");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"   \n  \n  ")
        .expect("failed to write whitespace");

    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(
        !output.status.success(),
        "whitespace-only stdin should cause failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("empty input"),
        "error should mention empty input: {stderr}"
    );
}

#[test]
fn thread_new_can_create_link_immediately() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let create_rfc = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "rfc",
            "new",
            "Switch backend",
            "--body",
            "## Goal\nSwitch to a new backend.",
        ])
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
    let state = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(state.links.len(), 1);
    assert_eq!(state.links[0].target_thread_id, "RFC-0001");
    assert_eq!(state.links[0].rel, "implements");
}

#[test]
fn from_thread_without_title_uses_default() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    // Create source RFC and move to accepted (so auto-deprecation works)
    let git = GitOps::new(repo.path().to_path_buf());
    let clock = SystemClock;
    let empty_policy = Policy::default();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Original design",
        Some("Body of original RFC"),
        "human/alice",
        &clock,
    )
    .unwrap();
    state_change::change_state(
        &git,
        "RFC-0001",
        "proposed",
        &[],
        "human/alice",
        &clock,
        &empty_policy,
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        "RFC-0001",
        "under-review",
        &[],
        "human/alice",
        &clock,
        &empty_policy,
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        "RFC-0001",
        "accepted",
        &[],
        "human/alice",
        &clock,
        &empty_policy,
        state_change::StateChangeOptions::default(),
    )
    .unwrap();

    // Create new RFC from source without explicit title (regression for finding #2)
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["rfc", "new", "--from-thread", "RFC-0001"])
        .output()
        .expect("failed to run git-forum rfc new --from-thread");
    assert!(
        output.status.success(),
        "from-thread without title should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let state = thread::replay_thread(&git, "RFC-0002").unwrap();
    assert_eq!(state.title, "v2: Original design");
    assert_eq!(state.body.as_deref(), Some("Body of original RFC"));
}

#[test]
fn from_thread_issue_to_issue_does_not_deprecate_source() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let git = GitOps::new(repo.path().to_path_buf());
    let clock = SystemClock;
    create::create_thread(
        &git,
        ThreadKind::Issue,
        "Original bug",
        Some("Body of original issue"),
        "human/alice",
        &clock,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "--from-thread", "ASK-0001"])
        .output()
        .expect("failed to run git-forum issue new --from-thread");
    assert!(
        output.status.success(),
        "issue --from-thread issue should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // New issue has links and copied content
    let new_state = thread::replay_thread(&git, "ASK-0002").unwrap();
    assert_eq!(new_state.title, "v2: Original bug");
    assert_eq!(new_state.body.as_deref(), Some("Body of original issue"));
    assert_eq!(new_state.links.len(), 1);
    assert_eq!(new_state.links[0].target_thread_id, "ASK-0001");
    assert_eq!(new_state.links[0].rel, "supersedes");

    // Source issue is NOT deprecated — remains in its prior state
    let source = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(source.status, "open");
    assert_eq!(source.links.len(), 1);
    assert_eq!(source.links[0].target_thread_id, "ASK-0002");
    assert_eq!(source.links[0].rel, "superseded-by");
}

#[test]
fn from_thread_issue_to_rfc_does_not_deprecate_source() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let git = GitOps::new(repo.path().to_path_buf());
    let clock = SystemClock;
    create::create_thread(
        &git,
        ThreadKind::Issue,
        "Feature request",
        Some("We need a better API"),
        "human/alice",
        &clock,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["rfc", "new", "--from-thread", "ASK-0001"])
        .output()
        .expect("failed to run git-forum rfc new --from-thread");
    assert!(
        output.status.success(),
        "rfc --from-thread issue should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // New RFC has links and copied content
    let new_state = thread::replay_thread(&git, "RFC-0001").unwrap();
    assert_eq!(new_state.title, "v2: Feature request");
    assert_eq!(new_state.body.as_deref(), Some("We need a better API"));
    assert_eq!(new_state.links.len(), 1);
    assert_eq!(new_state.links[0].target_thread_id, "ASK-0001");
    assert_eq!(new_state.links[0].rel, "supersedes");

    // Source issue is NOT deprecated
    let source = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(source.status, "open");
    assert_eq!(source.links.len(), 1);
    assert_eq!(source.links[0].target_thread_id, "RFC-0001");
    assert_eq!(source.links[0].rel, "superseded-by");
}

#[test]
fn from_thread_rfc_to_issue_is_rejected() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let git = GitOps::new(repo.path().to_path_buf());
    let clock = SystemClock;
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Some RFC",
        Some("RFC body"),
        "human/alice",
        &clock,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "--from-thread", "RFC-0001"])
        .output()
        .expect("failed to run git-forum issue new --from-thread");
    assert!(
        !output.status.success(),
        "issue --from-thread RFC should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot create an issue --from-thread an RFC"),
        "error should explain why: {stderr}"
    );
}
