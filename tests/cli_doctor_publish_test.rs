//! Doctor advisories for the published namespace per RFC `fls856j3`
//! §5 / §7.

mod support;

use std::path::Path;
use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;

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

fn create_issue(repo: &Path, title: &str) -> String {
    let out = run_forum(repo, &["new", "issue", title]);
    assert!(
        out.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    extract_created_id(&out)
}

fn doctor_output(repo: &Path) -> String {
    let out = run_forum(repo, &["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    format!("{stdout}{stderr}")
}

#[test]
fn auth_without_published_emits_info_advisory() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let id = create_issue(repo.path(), "Public, never pushed");
    let _ = run_forum(repo.path(), &["thread", "set-visibility", &id, "public"]);

    // No `git forum push` yet — published ref is absent.
    let report = doctor_output(repo.path());
    let want_marker = format!("auth-without-published {id}");
    assert!(
        report.contains(&want_marker),
        "missing advisory '{want_marker}' in:\n{report}"
    );
    assert!(
        report.contains("INFO"),
        "advisory should be tagged INFO:\n{report}"
    );
}

#[test]
fn stale_published_for_orphan_ref_warns() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    let id = create_issue(repo.path(), "About to be orphaned");
    let _ = run_forum(repo.path(), &["thread", "set-visibility", &id, "public"]);

    // Materialise a published ref locally without any remote: just
    // do the local-side of `git forum push` work via our own tool,
    // then delete the authoritative ref. We don't need a remote for
    // this test — the published ref is there from a synthetic
    // setup. Use git update-ref to seed it from the authoritative
    // tip's tree.
    let auth_tip = run_git(
        repo.path(),
        &["rev-parse", &format!("refs/forum/threads/{id}")],
    );
    assert!(auth_tip.status.success());
    let auth_tip_sha = String::from_utf8_lossy(&auth_tip.stdout).trim().to_string();
    let tree = run_git(
        repo.path(),
        &["rev-parse", &format!("{auth_tip_sha}^{{tree}}")],
    );
    assert!(tree.status.success());
    let tree_sha = String::from_utf8_lossy(&tree.stdout).trim().to_string();
    let cm = run_git(
        repo.path(),
        &[
            "commit-tree",
            &tree_sha,
            "-m",
            &format!("published snapshot of {id}"),
        ],
    );
    assert!(cm.status.success());
    let pub_sha = String::from_utf8_lossy(&cm.stdout).trim().to_string();
    let r = run_git(
        repo.path(),
        &[
            "update-ref",
            &format!("refs/forum/published/{id}"),
            &pub_sha,
        ],
    );
    assert!(r.status.success());

    // Now delete the authoritative ref to simulate an orphan
    // published ref.
    let d = run_git(
        repo.path(),
        &["update-ref", "-d", &format!("refs/forum/threads/{id}")],
    );
    assert!(d.status.success());

    let report = doctor_output(repo.path());
    let want_marker = format!("stale-published {id}");
    assert!(
        report.contains(&want_marker),
        "missing advisory '{want_marker}' in:\n{report}"
    );
    assert!(report.contains("WARN"), "expected WARN tag: {report}");
}
