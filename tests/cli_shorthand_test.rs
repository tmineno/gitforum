//! `git forum comment | objection | action` CLI shorthands.
//!
//! task `1hg98odf`: the rhetorical aliases
//! (`claim`, `question`, `summary`, `risk`, `review`) were deleted.
//! SPEC-3.0 §2.2 keeps only the four canonical NodeKinds.
//! This file's legacy event-chain test
//! (`question_command_creates_question_node`) is gone with the arm.
//!
//! Ticket `h36at1ti`: `--body -` and `--body-file -` both read piped
//! content from stdin on the four node-creating commands (`comment`,
//! `objection`, `action`, `node add`). Coverage below pipes a
//! multi-line body and asserts the resulting node body matches.

#[allow(dead_code)]
mod support;

use std::io::Write;
use std::process::{Command, Stdio};

use git_forum::internal::git_ops::GitOps;
use git_forum::internal::thread;

use support::cli::{fresh_repo, make_thread_via_cli};

/// Pipe `body` into `git forum <args>...` and return the process output.
fn run_with_stdin(repo_path: &std::path::Path, args: &[&str], body: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn git-forum");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(body)
        .expect("failed to write stdin");
    child.wait_with_output().expect("failed to wait on child")
}

const PIPED_BODY: &[u8] = b"first line of pipe\nsecond line of pipe\nthird line of pipe\n";
const PIPED_BODY_STR: &str = "first line of pipe\nsecond line of pipe\nthird line of pipe\n";

/// Return the body of the most-recently-added node on `thread_id`.
fn last_node_body(repo_path: &std::path::Path, thread_id: &str) -> String {
    let git = GitOps::new(repo_path.to_path_buf());
    let state = thread::replay_thread(&git, thread_id).expect("replay");
    state.nodes.last().expect("at least one node").body.clone()
}

#[test]
fn comment_body_dash_reads_stdin() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "issue", "Pipe target", "stub");
    let out = run_with_stdin(repo.path(), &["comment", &id, "--body", "-"], PIPED_BODY);
    assert!(
        out.status.success(),
        "comment --body - failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(last_node_body(repo.path(), &id), PIPED_BODY_STR);
}

#[test]
fn comment_body_file_dash_reads_stdin() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "issue", "Pipe target", "stub");
    let out = run_with_stdin(
        repo.path(),
        &["comment", &id, "--body-file", "-"],
        PIPED_BODY,
    );
    assert!(
        out.status.success(),
        "comment --body-file - failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(last_node_body(repo.path(), &id), PIPED_BODY_STR);
}

#[test]
fn objection_body_dash_reads_stdin() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "rfc", "Pipe target", "stub");
    let out = run_with_stdin(repo.path(), &["objection", &id, "--body", "-"], PIPED_BODY);
    assert!(
        out.status.success(),
        "objection --body - failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(last_node_body(repo.path(), &id), PIPED_BODY_STR);
}

#[test]
fn objection_body_file_dash_reads_stdin() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "rfc", "Pipe target", "stub");
    let out = run_with_stdin(
        repo.path(),
        &["objection", &id, "--body-file", "-"],
        PIPED_BODY,
    );
    assert!(
        out.status.success(),
        "objection --body-file - failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(last_node_body(repo.path(), &id), PIPED_BODY_STR);
}

#[test]
fn action_body_dash_reads_stdin() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "issue", "Pipe target", "stub");
    let out = run_with_stdin(repo.path(), &["action", &id, "--body", "-"], PIPED_BODY);
    assert!(
        out.status.success(),
        "action --body - failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(last_node_body(repo.path(), &id), PIPED_BODY_STR);
}

#[test]
fn action_body_file_dash_reads_stdin() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "issue", "Pipe target", "stub");
    let out = run_with_stdin(
        repo.path(),
        &["action", &id, "--body-file", "-"],
        PIPED_BODY,
    );
    assert!(
        out.status.success(),
        "action --body-file - failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(last_node_body(repo.path(), &id), PIPED_BODY_STR);
}

#[test]
fn node_add_body_dash_reads_stdin() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "issue", "Pipe target", "stub");
    let out = run_with_stdin(
        repo.path(),
        &["node", "add", &id, "--type", "comment", "--body", "-"],
        PIPED_BODY,
    );
    assert!(
        out.status.success(),
        "node add --body - failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(last_node_body(repo.path(), &id), PIPED_BODY_STR);
}

#[test]
fn node_add_body_file_dash_reads_stdin() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "issue", "Pipe target", "stub");
    let out = run_with_stdin(
        repo.path(),
        &["node", "add", &id, "--type", "comment", "--body-file", "-"],
        PIPED_BODY,
    );
    assert!(
        out.status.success(),
        "node add --body-file - failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(last_node_body(repo.path(), &id), PIPED_BODY_STR);
}

/// Acceptance criterion: `--edit` + stdin (`--body -`) is rejected at
/// the CLI surface. Clap already declares `--edit` as conflicting with
/// `--body` and `--body-file`, so we only need to confirm the rejection.
#[test]
fn edit_and_body_dash_are_mutually_exclusive() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "issue", "Conflict target", "stub");
    let out = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["comment", &id, "--edit", "--body", "-"])
        .output()
        .expect("failed to run git-forum comment");
    assert!(
        !out.status.success(),
        "--edit and --body - must be mutually exclusive"
    );
}

/// Same for `--edit` + `--body-file -`.
#[test]
fn edit_and_body_file_dash_are_mutually_exclusive() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "issue", "Conflict target", "stub");
    let out = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["comment", &id, "--edit", "--body-file", "-"])
        .output()
        .expect("failed to run git-forum comment");
    assert!(
        !out.status.success(),
        "--edit and --body-file - must be mutually exclusive"
    );
}
