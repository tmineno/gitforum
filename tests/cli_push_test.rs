//! `git forum push` integration tests (RFC `fls856j3`).
//!
//! Each test wires a bare git repository as `origin`, exercises the
//! publisher, and asserts the remote ends up in the right state.

mod support;

use std::path::Path;
use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;
use tempfile::TempDir;

use support::cli::extract_created_id;

/// Strip the host's git env from `cmd`. Pre-commit (which exports
/// GIT_DIR pointing at the parent repo) otherwise silently
/// retargets every spawned `git`/`git-forum` invocation to the wrong
/// place. Mirrors the isolation already done by
/// `support::repo::TestRepo` for git init / git config.
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
    isolate(&mut cmd).output().expect("git invocation failed")
}

fn run_forum(repo: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_git-forum"));
    cmd.current_dir(repo).args(args);
    isolate(&mut cmd)
        .output()
        .expect("git-forum invocation failed")
}

fn init_bare_remote() -> TempDir {
    let dir = TempDir::new().unwrap();
    let out = run_git(dir.path(), &["init", "--bare", "-q"]);
    assert!(out.status.success());
    dir
}

fn create_issue(repo_path: &Path, title: &str) -> String {
    let output = run_forum(repo_path, &["new", "issue", title]);
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    extract_created_id(&output)
}

fn set_visibility(repo_path: &Path, id: &str, value: &str, force: bool) {
    let mut args: Vec<&str> = vec!["thread", "set-visibility", id, value];
    if force {
        args.push("--force");
    }
    let out = run_forum(repo_path, &args);
    assert!(
        out.status.success(),
        "set-visibility failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn add_origin(repo_path: &Path, remote_url: &Path) {
    let out = run_git(
        repo_path,
        &[
            "remote",
            "add",
            "origin",
            remote_url.to_str().expect("utf8 path"),
        ],
    );
    assert!(out.status.success(), "add origin failed");
}

fn ls_remote_published(remote_path: &Path) -> Vec<String> {
    let out = run_git(
        remote_path,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/forum/published/",
        ],
    );
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn push_creates_published_refs_for_public_only() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let pub_id = create_issue(repo.path(), "Public");
    let priv_id = create_issue(repo.path(), "Private");
    set_visibility(repo.path(), &pub_id, "public", false);
    // priv_id stays at default (private).

    let bare = init_bare_remote();
    add_origin(repo.path(), bare.path());

    let push = run_forum(repo.path(), &["push"]);
    assert!(
        push.status.success(),
        "push failed: {}",
        String::from_utf8_lossy(&push.stderr)
    );
    let stdout = String::from_utf8_lossy(&push.stdout);
    assert!(
        stdout.contains("Published 1 threads (1 new"),
        "summary missing in: {stdout}"
    );

    let remote_refs = ls_remote_published(bare.path());
    let want = format!("refs/forum/published/{pub_id}");
    let unwanted = format!("refs/forum/published/{priv_id}");
    assert!(remote_refs.contains(&want), "{remote_refs:?}");
    assert!(!remote_refs.contains(&unwanted), "{remote_refs:?}");
}

#[test]
fn second_push_is_a_no_op() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let id = create_issue(repo.path(), "Public");
    set_visibility(repo.path(), &id, "public", false);

    let bare = init_bare_remote();
    add_origin(repo.path(), bare.path());

    run_forum(repo.path(), &["push"]);

    let push2 = run_forum(repo.path(), &["push"]);
    assert!(push2.status.success());
    let stdout = String::from_utf8_lossy(&push2.stdout);
    assert!(
        stdout.contains("Published 0 threads"),
        "expected no-op summary, got: {stdout}"
    );
}

#[test]
fn flip_to_private_withdraws_remote_ref() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let id = create_issue(repo.path(), "Soon-to-be-private");
    set_visibility(repo.path(), &id, "public", false);

    let bare = init_bare_remote();
    add_origin(repo.path(), bare.path());

    // Publish.
    let p1 = run_forum(repo.path(), &["push"]);
    assert!(p1.status.success());
    assert!(ls_remote_published(bare.path()).contains(&format!("refs/forum/published/{id}")));

    // Flip and push again → withdrawal.
    set_visibility(repo.path(), &id, "private", true);
    let p2 = run_forum(repo.path(), &["push"]);
    assert!(
        p2.status.success(),
        "push withdrawal failed: {}",
        String::from_utf8_lossy(&p2.stderr)
    );
    let stdout = String::from_utf8_lossy(&p2.stdout);
    assert!(
        stdout.contains("1 withdrawn"),
        "summary missing withdrawal: {stdout}"
    );
    assert!(!ls_remote_published(bare.path()).contains(&format!("refs/forum/published/{id}")));
    // Local published ref also gone after remote ack.
    let local_check = run_git(
        repo.path(),
        &[
            "rev-parse",
            "--verify",
            &format!("refs/forum/published/{id}"),
        ],
    );
    assert!(
        !local_check.status.success(),
        "local published should be deleted"
    );
}

#[test]
fn lint_warning_with_strict_exits_non_zero() {
    // Public thread mentions a private thread by id → lint warning.
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let priv_id = create_issue(repo.path(), "Private");
    // Body must include the private id verbatim.
    let body = format!("body that references @{priv_id}");
    let create = run_forum(repo.path(), &["new", "issue", "Public", "--body", &body]);
    assert!(create.status.success());
    let pub_id = extract_created_id(&create);
    set_visibility(repo.path(), &pub_id, "public", false);

    let bare = init_bare_remote();
    add_origin(repo.path(), bare.path());

    // Without --strict: succeeds, prints warning to stderr.
    let lax = run_forum(repo.path(), &["push"]);
    assert!(lax.status.success());
    let stderr = String::from_utf8_lossy(&lax.stderr);
    assert!(
        stderr.contains(&format!("at-id:{priv_id}")),
        "expected lint warning in stderr: {stderr}"
    );

    // Reset remote so a re-push has work.
    drop(bare);
    let bare2 = init_bare_remote();
    let out = run_git(repo.path(), &["remote", "remove", "origin"]);
    assert!(out.status.success());
    add_origin(repo.path(), bare2.path());
    // Force a re-publish by deleting the local published ref.
    run_git(
        repo.path(),
        &[
            "update-ref",
            "-d",
            &format!("refs/forum/published/{pub_id}"),
        ],
    );

    // With --strict: exits non-zero.
    let strict = run_forum(repo.path(), &["push", "--strict"]);
    assert!(!strict.status.success(), "strict mode should exit non-zero");
}

#[test]
fn strict_failure_does_not_advance_local_or_remote() {
    // RFC §5.5 "lint before build": --strict must not write any
    // local published ref or push anything when warnings exist.
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let priv_id = create_issue(repo.path(), "Private");
    let body = format!("see @{priv_id}");
    let create = run_forum(repo.path(), &["new", "issue", "Public", "--body", &body]);
    assert!(create.status.success());
    let pub_id = extract_created_id(&create);
    set_visibility(repo.path(), &pub_id, "public", false);

    let bare = init_bare_remote();
    add_origin(repo.path(), bare.path());

    // Strict push should fail and leave local + remote untouched.
    let strict = run_forum(repo.path(), &["push", "--strict"]);
    assert!(!strict.status.success(), "strict should exit non-zero");

    // No local published ref was created.
    let local = run_git(
        repo.path(),
        &[
            "rev-parse",
            "--verify",
            &format!("refs/forum/published/{pub_id}"),
        ],
    );
    assert!(
        !local.status.success(),
        "strict failure must not write a local published ref"
    );
    // Remote received nothing.
    assert!(
        ls_remote_published(bare.path()).is_empty(),
        "strict failure must not push anything"
    );
}

#[test]
fn skipped_entries_are_re_pushed_after_remote_failure() {
    // RFC §5.6 retry semantics: when a previous push failed remotely,
    // a re-push must include the otherwise-skipped refspec so the
    // remote catches up. We simulate the failure by deleting the
    // remote ref directly and re-running push — the local ref
    // matches the source tree so build_plan would mark it Skipped,
    // but refspecs() still emits +REF:REF.
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let id = create_issue(repo.path(), "Public");
    set_visibility(repo.path(), &id, "public", false);

    let bare = init_bare_remote();
    add_origin(repo.path(), bare.path());

    let p1 = run_forum(repo.path(), &["push"]);
    assert!(p1.status.success());
    assert!(ls_remote_published(bare.path()).contains(&format!("refs/forum/published/{id}")));

    // Simulate a partial failure: remote forgets the published ref,
    // local still has it.
    let del = run_git(
        bare.path(),
        &["update-ref", "-d", &format!("refs/forum/published/{id}")],
    );
    assert!(del.status.success());
    assert!(ls_remote_published(bare.path()).is_empty());

    // Second push: tree-equivalence-skip locally, but the refspec
    // is still emitted so the remote ref is restored.
    let p2 = run_forum(repo.path(), &["push"]);
    assert!(
        p2.status.success(),
        "retry push failed: {}",
        String::from_utf8_lossy(&p2.stderr)
    );
    assert!(
        ls_remote_published(bare.path()).contains(&format!("refs/forum/published/{id}")),
        "remote published ref must be restored on retry"
    );
}
