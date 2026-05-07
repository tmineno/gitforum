//! `git forum show` / `ls` read fallback to refs/forum/published/*
//! per RFC `fls856j3` §5.
//!
//! These tests exercise the fallback path used by public-consumer
//! clones (`git forum init --public-only`): reads must succeed
//! against the published-namespace ref when the authoritative ref
//! is absent.

mod support;

use std::path::Path;
use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;
use tempfile::TempDir;

use support::cli::extract_created_id;

fn isolate(cmd: &mut Command) -> &mut Command {
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
}

fn run_git(repo: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo).args(args);
    isolate(&mut cmd).output().expect("git failed")
}

fn run_forum(repo: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_git-forum"));
    cmd.current_dir(repo).args(args);
    isolate(&mut cmd).output().expect("git-forum failed")
}

fn create_issue(repo_path: &Path, title: &str, body: &str) -> String {
    let output = run_forum(repo_path, &["new", "issue", title, "--body", body]);
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    extract_created_id(&output)
}

fn set_public(repo_path: &Path, id: &str) {
    let out = run_forum(repo_path, &["thread", "set-visibility", id, "public"]);
    assert!(out.status.success());
}

fn init_bare_remote() -> TempDir {
    let dir = TempDir::new().unwrap();
    let out = run_git(dir.path(), &["init", "--bare", "-q"]);
    assert!(out.status.success());
    dir
}

#[test]
fn show_falls_back_to_published_when_authoritative_absent() {
    // Set up a publisher repo with one public thread.
    let publisher = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(publisher.path());
    init::init_forum(&paths).unwrap();
    let id = create_issue(publisher.path(), "Goes public", "Body content here.");
    set_public(publisher.path(), &id);

    let bare = init_bare_remote();
    let bare_url = bare.path().to_str().unwrap();
    run_git(publisher.path(), &["remote", "add", "origin", bare_url]);
    let push = run_forum(publisher.path(), &["push"]);
    assert!(
        push.status.success(),
        "push failed: {}",
        String::from_utf8_lossy(&push.stderr)
    );

    // Build a published-only consumer: copy the bare into a fresh
    // working clone, set up --public-only, fetch.
    let consumer = TempDir::new().unwrap();
    let clone_status = Command::new("git")
        .args(["clone", "-q", bare_url, "consumer"])
        .current_dir(consumer.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(clone_status.success());
    let consumer_path = consumer.path().join("consumer");
    // Configure git identity in the clone so any later forum
    // commit (e.g. a hook) doesn't fail.
    run_git(
        &consumer_path,
        &["config", "user.email", "test@example.com"],
    );
    run_git(&consumer_path, &["config", "user.name", "Test User"]);

    // Init in --public-only mode.
    let init_out = run_forum(&consumer_path, &["init", "--public-only"]);
    assert!(
        init_out.status.success(),
        "init --public-only failed: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );

    // The consumer must NOT have the authoritative ref, only the
    // published one.
    assert!(
        !run_git(
            &consumer_path,
            &["rev-parse", "--verify", &format!("refs/forum/threads/{id}")],
        )
        .status
        .success(),
        "consumer must not have authoritative ref"
    );
    let pub_ref = format!("refs/forum/published/{id}");
    assert!(
        run_git(&consumer_path, &["rev-parse", "--verify", &pub_ref])
            .status
            .success(),
        "consumer should have published ref {pub_ref}"
    );

    // `git forum show` falls back to published.
    let show = run_forum(&consumer_path, &["show", &id]);
    assert!(
        show.status.success(),
        "show failed: {}",
        String::from_utf8_lossy(&show.stderr)
    );
    let stdout = String::from_utf8_lossy(&show.stdout);
    assert!(
        stdout.contains("Goes public"),
        "show output missing title: {stdout}"
    );

    // `git forum ls` lists the thread by id.
    let ls = run_forum(&consumer_path, &["ls"]);
    assert!(ls.status.success());
    let ls_out = String::from_utf8_lossy(&ls.stdout);
    assert!(ls_out.contains(&id), "ls missing id {id}: {ls_out}");

    // RFC §5.5: write paths must NOT synthesize a fresh
    // authoritative ref from the published-only state. A
    // visibility flip on the consumer (which has only the published
    // ref) must be rejected before `refs/forum/threads/<id>` gets
    // written from sanitized data.
    let flip = run_forum(
        &consumer_path,
        &["thread", "set-visibility", &id, "private", "--force"],
    );
    assert!(
        !flip.status.success(),
        "set-visibility on published-only thread must fail; stdout={} stderr={}",
        String::from_utf8_lossy(&flip.stdout),
        String::from_utf8_lossy(&flip.stderr)
    );
    let stderr = String::from_utf8_lossy(&flip.stderr);
    assert!(
        stderr.contains("refusing to create"),
        "expected synthesize-guard error, got: {stderr}"
    );
    // No authoritative ref was created on the consumer.
    assert!(
        !run_git(
            &consumer_path,
            &["rev-parse", "--verify", &format!("refs/forum/threads/{id}")],
        )
        .status
        .success(),
        "consumer must still have no authoritative ref after the rejected write"
    );
}
