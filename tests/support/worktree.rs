#![allow(dead_code)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use git_forum::internal::git_ops::GitOps;

use super::scenario::ActorDef;

/// Create a git worktree for a simulated actor.
///
/// - Creates a seed commit in `main_repo` if it has no commits yet.
/// - Runs `git worktree add <base_dir>/<actor> -b actor/<actor>`.
/// - Returns the worktree path and a `GitOps` bound to it.
pub fn create_actor_worktree(main_repo: &Path, actor: &str, base_dir: &Path) -> (PathBuf, GitOps) {
    ensure_seed_commit(main_repo);
    let wt_path = base_dir.join(actor);
    let branch = format!("actor/{actor}");
    let output = Command::new("git")
        .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", &branch])
        .current_dir(main_repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git worktree add failed");
    assert!(
        output.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (wt_path.clone(), GitOps::new(wt_path))
}

/// Commit the .forum/ directory so it's visible to worktrees.
pub fn commit_forum_config(repo: &Path) {
    let output = Command::new("git")
        .args(["add", ".forum"])
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git add .forum failed");
    assert!(
        output.status.success(),
        "git add .forum failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = Command::new("git")
        .args(["commit", "-m", "commit forum config"])
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git commit .forum failed");
    assert!(
        output.status.success(),
        "git commit .forum failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Set up worktrees for all actors in a scenario.
///
/// Returns a map of actor name -> (worktree_path, GitOps).
pub fn setup_actor_worktrees(
    main_repo: &Path,
    actors: &[ActorDef],
    base_dir: &Path,
) -> HashMap<String, (PathBuf, GitOps)> {
    commit_forum_config(main_repo);
    let mut result = HashMap::new();
    for actor in actors {
        let (path, git) = create_actor_worktree(main_repo, &actor.name, base_dir);
        result.insert(actor.name.clone(), (path, git));
    }
    result
}

/// Ensure the repo has at least one commit (required for worktrees).
fn ensure_seed_commit(repo: &Path) {
    let check = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git rev-parse failed");
    if check.status.success() {
        return; // already has commits
    }
    // Create an empty initial commit
    let output = Command::new("git")
        .args(["commit", "--allow-empty", "-m", "seed commit"])
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git commit failed");
    assert!(
        output.status.success(),
        "seed commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
