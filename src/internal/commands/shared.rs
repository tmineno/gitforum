//! Helpers shared across `commands::*` orchestration modules.
//!
//! These wrap small pieces that previously lived in `main.rs` and were used
//! by every `run_*` function: repo discovery with the init warning,
//! operation-check application, actor/thread-id resolution. Kept here (not
//! re-introduced as a Service / DTO layer per #yjelk0s0 Out-of-scope) so
//! command modules don't need a back-reference to `main.rs`.

use crate::internal::actor;
use crate::internal::config::{self, RepoPaths};
use crate::internal::error::ForumError;
use crate::internal::git_ops::GitOps;
use crate::internal::operation_check;
use crate::internal::thread;

/// Apply the result of an operation-check pass: print violations to stderr,
/// and return a `Policy` error if any are blocking. `force` and `strict`
/// flow from the CLI flags / policy config.
pub fn apply_operation_checks(
    violations: &[operation_check::OperationViolation],
    force: bool,
    strict: bool,
) -> Result<(), ForumError> {
    if violations.is_empty() {
        return Ok(());
    }
    let (has_errors, output) = operation_check::evaluate_violations(violations, force, strict);
    eprint!("{output}");
    if has_errors {
        Err(ForumError::Policy(
            "operation blocked by check violations".into(),
        ))
    } else {
        Ok(())
    }
}

/// Discover the surrounding git repo and `.forum/` paths, warning the user
/// when the repo has not been initialised yet. Returns the `GitOps` handle
/// (with commit identity / default actor pre-loaded from local config) and
/// the resolved `RepoPaths`.
pub fn discover_repo_with_init_warning() -> Result<(GitOps, RepoPaths), ForumError> {
    let mut git = GitOps::discover()?;
    let git_dir = git.git_dir()?;
    let paths = RepoPaths::from_repo_root_and_git_dir(git.root(), &git_dir);
    if !is_forum_initialized(&paths, &git) {
        eprintln!(
            "warning: git-forum is not initialized in this repository; run `git forum init` first"
        );
    }
    let local_cfg = config::load_local_config(&paths).unwrap_or_default();
    if let Some(identity) = local_cfg.commit_identity {
        git.set_commit_identity(identity);
    }
    if let Some(default_actor) = local_cfg.default_actor {
        git.set_default_actor(default_actor);
    }
    Ok((git, paths))
}

fn is_forum_initialized(paths: &RepoPaths, git: &GitOps) -> bool {
    if paths.dot_forum.join("policy.toml").is_file() && paths.git_forum.join("logs").is_dir() {
        return true;
    }
    git.list_refs("refs/forum/threads/")
        .map(|refs| !refs.is_empty())
        .unwrap_or(false)
}

/// Resolve the effective actor for a CLI write: prefer `--as`, otherwise
/// fall back to the local actor config (defaulted from git identity).
pub fn resolve_actor(as_actor: Option<String>, git: &GitOps) -> String {
    as_actor.unwrap_or_else(|| actor::current_actor(git, git.default_actor()))
}

/// Resolve a user-supplied thread reference to its canonical full ID.
/// Wraps `thread::resolve_thread_id` for use from CLI command handlers.
pub fn resolve_tid(git: &GitOps, user_input: &str) -> Result<String, ForumError> {
    thread::resolve_thread_id(git, user_input)
}
