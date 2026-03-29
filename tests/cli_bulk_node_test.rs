mod support;

use std::process::Command;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;
use git_forum::internal::write_ops;

fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

#[test]
fn retract_multiple_nodes() {
    let (repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Bulk retract test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let n1 = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "Summary 1",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let n2 = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "Summary 2",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["retract", &thread_id, &n1, &n2])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "retract multi failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Retracted"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.nodes[0].retracted);
    assert!(state.nodes[1].retracted);
}

#[test]
fn resolve_multiple_nodes() {
    let (repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Bulk resolve test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let n1 = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Objection 1",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let n2 = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Objection 2",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["resolve", &thread_id, &n1, &n2])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "resolve multi failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Resolved"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.open_objections().is_empty());
}

#[test]
fn reopen_multiple_nodes() {
    let (repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Bulk reopen test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let n1 = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Objection 1",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let n2 = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Objection 2",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    // Resolve both first
    write_ops::resolve_node(&git, &thread_id, &n1, "human/alice", &fixed_clock()).unwrap();
    write_ops::resolve_node(&git, &thread_id, &n2, "human/alice", &fixed_clock()).unwrap();

    // Reopen both
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["reopen", &thread_id, &n1, &n2])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "reopen multi failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Reopened"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.open_objections().len(), 2);
}

#[test]
fn single_node_id_still_works() {
    let (repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Single node test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let n1 = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Action,
        "Do something",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["resolve", &thread_id, &n1])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "single resolve failed");
    assert!(stdout.contains("Resolved"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.nodes[0].resolved);
}

#[test]
fn bulk_retract_reports_failures_inline() {
    let (repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Failure test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let n1 = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "Good node",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    // Use a valid node and a bogus one
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["retract", &thread_id, &n1, "bogus_node_id_does_not_exist"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should exit non-zero
    assert!(!output.status.success());
    // The valid node should still be retracted
    assert!(stdout.contains("Retracted"));
    // The bogus one should report an error
    assert!(stderr.contains("error:"));

    // Verify the valid node was retracted despite the failure
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.nodes[0].retracted);
}

#[test]
fn reopen_without_node_ids_is_rejected() {
    let (repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Thread reopen test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    // Close the thread
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["close", &thread_id])
        .output()
        .expect("failed to run");
    assert!(output.status.success());

    // Reopen without node IDs should reopen the thread itself
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["reopen", &thread_id])
        .output()
        .expect("failed to run");
    assert!(
        output.status.success(),
        "reopen without node IDs should reopen the thread: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "open");
}

#[test]
fn thread_reopen_via_issue_subcommand() {
    let (repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Thread reopen test",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    // Close the thread
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["close", &thread_id])
        .output()
        .expect("failed to run");
    assert!(output.status.success());

    // Reopen via issue subcommand
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "reopen", &thread_id])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "thread reopen via issue subcommand failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("-> open"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "open");
}
