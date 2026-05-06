//! `git forum thread set-visibility` integration tests.
//!
//! Covers RFC `fls856j3` §6: round-trip flips, no-op when value
//! unchanged, and the irrevocability guard on `public → private` from
//! a non-interactive shell.

mod support;

use std::process::{Command, Stdio};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::snapshot;
use git_forum::internal::thread::Visibility;

use support::cli::extract_created_id;

fn create_thread(repo_path: &std::path::Path, title: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo_path)
        .args(["new", "issue", title])
        .output()
        .expect("failed to create rfc");
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    extract_created_id(&output)
}

#[test]
fn set_visibility_public_then_private_round_trips() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let id = create_thread(repo.path(), "Visibility round-trip");

    // New threads default to private (absent on disk).
    let git = GitOps::new(repo.path().to_path_buf());
    let doc = snapshot::read_snapshot(&git, &id).unwrap();
    assert_eq!(doc.snapshot.visibility, Visibility::Private);

    // Flip to public.
    let to_public = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["thread", "set-visibility", &id, "public"])
        .output()
        .expect("failed to set-visibility public");
    assert!(
        to_public.status.success(),
        "set-visibility public failed: {}",
        String::from_utf8_lossy(&to_public.stderr)
    );
    let doc = snapshot::read_snapshot(&git, &id).unwrap();
    assert_eq!(doc.snapshot.visibility, Visibility::Public);

    // Flip back to private with --force (test runs without a TTY).
    let to_private = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["thread", "set-visibility", &id, "private", "--force"])
        .output()
        .expect("failed to set-visibility private --force");
    assert!(
        to_private.status.success(),
        "set-visibility private --force failed: {}",
        String::from_utf8_lossy(&to_private.stderr)
    );
    let doc = snapshot::read_snapshot(&git, &id).unwrap();
    assert_eq!(doc.snapshot.visibility, Visibility::Private);
}

#[test]
fn set_visibility_idempotent_when_unchanged() {
    // Asking for the current visibility must not write a new snapshot.
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let id = create_thread(repo.path(), "Idempotent visibility");

    let git = GitOps::new(repo.path().to_path_buf());
    let refname = format!("refs/forum/threads/{id}");
    let before = git.resolve_ref(&refname).unwrap().unwrap();

    // New thread is private; setting it private again must be a no-op.
    let again = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["thread", "set-visibility", &id, "private"])
        .output()
        .expect("failed to run set-visibility private");
    assert!(again.status.success());

    let after = git.resolve_ref(&refname).unwrap().unwrap();
    assert_eq!(
        before, after,
        "no-op set-visibility must not advance the thread ref"
    );
}

#[test]
fn public_to_private_without_force_fails_in_non_tty() {
    // RFC §6: `public → private` from a non-interactive shell
    // requires --force. Tests inherit a non-TTY stdin from cargo
    // (we explicitly null it to be safe across runners).
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let id = create_thread(repo.path(), "Force-required flip");

    // Make it public first.
    let to_public = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["thread", "set-visibility", &id, "public"])
        .output()
        .unwrap();
    assert!(to_public.status.success());

    // Try public → private without --force from a non-TTY shell.
    let bad = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["thread", "set-visibility", &id, "private"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        !bad.status.success(),
        "expected non-zero exit for non-TTY public → private without --force"
    );
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(
        stderr.contains("--force") || stderr.contains("force"),
        "expected mention of --force in error: {stderr}"
    );

    // Visibility must be unchanged on disk.
    let git = GitOps::new(repo.path().to_path_buf());
    let doc = snapshot::read_snapshot(&git, &id).unwrap();
    assert_eq!(doc.snapshot.visibility, Visibility::Public);
}

#[test]
fn invalid_visibility_string_rejected() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let id = create_thread(repo.path(), "Invalid value");

    let bad = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["thread", "set-visibility", &id, "secret"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(
        stderr.contains("invalid visibility")
            || stderr.contains("public")
            || stderr.contains("private"),
        "expected invalid-visibility hint in error: {stderr}"
    );
}
