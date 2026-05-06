mod support;

use std::process::Command;

#[test]
fn say_subcommand_shows_node_shorthand_hint() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("say")
        .output()
        .expect("failed to run git-forum say");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("node shorthands"),
        "should suggest node shorthands; stderr: {stderr}"
    );
    assert!(
        stderr.contains("git forum comment"),
        "should mention comment shorthand; stderr: {stderr}"
    );
}

#[test]
fn rfc_subcommand_shows_kind_removal_hint() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("rfc")
        .output()
        .expect("failed to run git-forum rfc");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("removed in 2.0"),
        "should mention 2.0 removal; stderr: {stderr}"
    );
    assert!(
        stderr.contains("git forum new <kind>"),
        "should redirect to top-level form; stderr: {stderr}"
    );
}

#[test]
fn revise_body_subcommand_shows_hint() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("revise-body")
        .output()
        .expect("failed to run git-forum revise-body");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("git forum revise"),
        "should suggest revise command; stderr: {stderr}"
    );
}

#[test]
fn create_subcommand_shows_hint() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("create")
        .output()
        .expect("failed to run git-forum create");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("git forum new"),
        "should suggest new command; stderr: {stderr}"
    );
}

#[test]
fn add_subcommand_shows_hint() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("add")
        .output()
        .expect("failed to run git-forum add");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("git forum node add"),
        "should suggest node add command; stderr: {stderr}"
    );
}

/// Bug `k4p0ya0b`: `git forum show <node-id>` historically dead-ended
/// at "thread '<id>' not found" because thread ids and node ids
/// share the same 8-char base36 shape. Now the show command falls
/// through to the node-id index and emits a friendly redirect.
#[test]
fn show_with_node_id_redirects_to_node_show() {
    let repo = support::cli::fresh_repo();

    let thread_id = support::cli::make_thread_via_cli(
        repo.path(),
        "task",
        "Parent thread for node-id-redirect",
        "Body for parent thread.",
    );

    let comment_out = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "comment",
            &thread_id,
            "Comment whose id will be pasted into show",
        ])
        .output()
        .expect("failed to run git-forum comment");
    assert!(
        comment_out.status.success(),
        "comment failed: {}",
        String::from_utf8_lossy(&comment_out.stderr)
    );

    // `comment` prints `Created node <node-id> on @<thread-id>`.
    let comment_stdout = String::from_utf8_lossy(&comment_out.stdout).to_string();
    let node_id = comment_stdout
        .split_whitespace()
        .nth(2)
        .expect("comment output should contain the node id")
        .to_string();

    let show_out = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["show", &node_id])
        .output()
        .expect("failed to run git-forum show");

    assert!(
        !show_out.status.success(),
        "show <node-id> should still exit non-zero (callers expect an error \
         exit while we keep `node show` as the canonical surface)"
    );

    let stderr = String::from_utf8(show_out.stderr).expect("stderr utf-8");
    assert!(
        stderr.contains("is a node, not a thread"),
        "stderr should name the input as a node; got: {stderr}"
    );
    assert!(
        stderr.contains(&format!("git forum node show {node_id}")),
        "stderr should redirect to `node show <node-id>`; got: {stderr}"
    );
    assert!(
        stderr.contains(&thread_id),
        "stderr should name the parent thread {thread_id}; got: {stderr}"
    );
}

/// Bug `k4p0ya0b` exception: ids that match neither a thread nor a
/// node must still surface the original thread-not-found error so
/// the user is told to run `git forum ls`.
#[test]
fn show_with_unknown_id_still_errors_thread_not_found() {
    let repo = support::cli::fresh_repo();
    // Seed one thread so the forum is initialised but our probe id
    // is genuinely unknown.
    support::cli::make_thread_via_cli(repo.path(), "task", "A real thread", "Body.");

    let show_out = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["show", "zzzzzzzz"])
        .output()
        .expect("failed to run git-forum show");

    assert!(
        !show_out.status.success(),
        "show on an unknown id should still fail"
    );

    let stderr = String::from_utf8(show_out.stderr).expect("stderr utf-8");
    assert!(
        stderr.contains("thread 'zzzzzzzz' not found"),
        "stderr should keep the thread-not-found message for unknown ids; got: {stderr}"
    );
    assert!(
        stderr.contains("git forum ls"),
        "stderr should keep the `git forum ls` hint for unknown ids; got: {stderr}"
    );
    assert!(
        !stderr.contains("is a node, not a thread"),
        "stderr should NOT misclassify an unknown id as a node; got: {stderr}"
    );
}

#[test]
fn unknown_subcommand_shows_help_llm_fallback() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("nonexistent")
        .output()
        .expect("failed to run git-forum nonexistent");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("--help-llm"),
        "should suggest --help-llm; stderr: {stderr}"
    );
}
