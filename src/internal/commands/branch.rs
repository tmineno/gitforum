//! `git forum branch {bind|clear}` orchestration.
//!
//! task `1hg98odf`: writes the `branch` field on
//! `thread.toml` directly via `snapshot::store::write_snapshot` per
//! SPEC-3.0 §3.1. The legacy `Scope` event-write path is no longer
//! invoked here.

use clap::Subcommand;

use super::super::clock::Clock;
use super::super::error::{ForumError, ForumResult};
use super::super::git_ops::GitOps;
use super::super::snapshot::{self, store::write_snapshot};
use super::context::Context;
use super::shared::{discover_repo_with_init_warning, resolve_actor, resolve_tid};

/// Sub-commands for `git forum branch`. Owns the clap surface so
/// main.rs's branch arm is a thin dispatcher.
#[derive(Subcommand)]
pub enum BranchCmd {
    /// Bind a thread to an existing Git branch
    Bind {
        thread_id: String,
        branch: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Clear the bound branch from a thread
    Clear {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
}

/// Uniform entry point for the `branch` subcommand per RFC `7ymtc4b2`
/// criterion 3 ("every public command exposes
/// `internal::commands::<cmd>::run(args, &ctx)`").
pub fn run(cmd: BranchCmd, ctx: &Context) -> ForumResult<()> {
    match cmd {
        BranchCmd::Bind {
            thread_id,
            branch,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let actor = resolve_actor(as_actor, &git);
            set_branch(&git, &thread_id, Some(&branch), &actor, ctx.clock.as_ref())?;
            println!("{thread_id} -> branch {branch}");
        }
        BranchCmd::Clear {
            thread_id,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let actor = resolve_actor(as_actor, &git);
            set_branch(&git, &thread_id, None, &actor, ctx.clock.as_ref())?;
            println!("{thread_id} -> branch <cleared>");
        }
    }
    Ok(())
}

/// Bind or clear a thread's branch scope.
///
/// Preconditions: thread_id exists; when `branch` is Some, the branch
/// exists in `refs/heads/`.
/// Postconditions: `thread.toml.branch` is updated to the new value
/// (or removed when clearing).
/// Failure modes: `ForumError::Repo` when the branch does not exist;
/// `ForumError::Git` on subprocess failure.
/// Side effects: writes one snapshot commit and updates the thread ref.
pub fn set_branch(
    git: &GitOps,
    thread_id: &str,
    branch: Option<&str>,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    if let Some(branch) = branch {
        let refname = format!("refs/heads/{branch}");
        if git.resolve_ref(&refname)?.is_none() {
            return Err(ForumError::Repo(format!(
                "branch '{branch}' does not exist in this repository"
            )));
        }
    }

    let mut doc = snapshot::read_snapshot(git, thread_id)?;
    let now = clock.now();
    doc.snapshot.branch = branch.map(String::from);
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.into();

    let msg = match branch {
        Some(b) => format!("branch bind {thread_id} -> {b}"),
        None => format!("branch clear {thread_id}"),
    };
    write_snapshot(git, thread_id, &doc, &msg)?;
    Ok(())
}
