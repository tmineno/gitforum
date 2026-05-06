//! `git forum thread set-visibility` orchestration.
//!
//! RFC `fls856j3` §6: toggles `thread.toml.visibility` between `public`
//! and `private`. The `private → public` transition is the explicit
//! allowlist step. The `public → private` transition warns about
//! irrevocability (anyone who has already fetched the published ref
//! retains a copy; see RFC §2 residual retention) and requires
//! `--force` in non-interactive runs.
//!
//! The flip is recorded immediately on the thread; the corresponding
//! `refs/forum/published/<id>` is removed on the next `git forum push`
//! per RFC §7.

use std::io::IsTerminal;

use super::super::clock::Clock;
use super::super::error::{ForumError, ForumResult};
use super::super::git_ops::GitOps;
use super::super::snapshot::{self, store::write_snapshot};
use super::super::thread::Visibility;
use super::context::Context;
use super::shared::{discover_repo_with_init_warning, resolve_actor, resolve_tid};

/// Args bundle for `set_visibility` so the main.rs dispatch is a thin
/// wrapper.
pub struct SetVisibilityArgs {
    pub thread_id: String,
    pub visibility: Visibility,
    pub as_actor: Option<String>,
    pub force: bool,
}

/// Uniform entry point per RFC `7ymtc4b2` ("every public command
/// exposes `internal::commands::<cmd>::run(args, &ctx)`").
pub fn run(args: SetVisibilityArgs, ctx: &Context) -> ForumResult<()> {
    let (git, _paths) = discover_repo_with_init_warning()?;
    let thread_id = resolve_tid(&git, &args.thread_id)?;
    let actor = resolve_actor(args.as_actor, &git);
    set_visibility(
        &git,
        &thread_id,
        args.visibility,
        args.force,
        &actor,
        ctx.clock.as_ref(),
    )?;
    let label = match args.visibility {
        Visibility::Public => "public",
        Visibility::Private => "private",
    };
    println!("{thread_id} -> visibility {label}");
    Ok(())
}

/// Set a thread's visibility.
///
/// Preconditions: thread_id exists.
/// Postconditions: `thread.toml.visibility` reflects the new value
/// (or is absent when `Private`, which deserializes back to `Private`).
/// Failure modes: `ForumError::Repo` when a `public → private` flip
/// is requested without `--force` on a non-interactive stdin.
/// Side effects: writes one snapshot commit and updates the thread
/// ref. Stale `refs/forum/published/<id>` (if any) is **not** removed
/// here; the next `git forum push` reconciles it (RFC §7).
pub fn set_visibility(
    git: &GitOps,
    thread_id: &str,
    new_visibility: Visibility,
    force: bool,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    let mut doc = snapshot::read_snapshot(git, thread_id)?;
    let current = doc.snapshot.visibility;

    if current == new_visibility {
        // No-op: avoid writing a snapshot commit when nothing changed.
        return Ok(());
    }

    if current == Visibility::Public && new_visibility == Visibility::Private {
        // public → private is irrevocable in the privacy sense:
        // anyone who already fetched the published ref keeps it.
        // Warn always; require --force when stdin is not a TTY so
        // scripts cannot flip without acknowledging.
        eprintln!(
            "warning: flipping {thread_id} public → private is irrevocable for already-published copies."
        );
        eprintln!(
            "         Forks and mirrors that have already fetched refs/forum/published/{thread_id}"
        );
        eprintln!("         retain the prior tree (see RFC fls856j3 §2 residual retention).");
        if !std::io::stdin().is_terminal() && !force {
            return Err(ForumError::Repo(format!(
                "refusing to flip {thread_id} public → private from a non-interactive shell without --force"
            )));
        }
    }

    let now = clock.now();
    doc.snapshot.visibility = new_visibility;
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.into();

    let label = match new_visibility {
        Visibility::Public => "public",
        Visibility::Private => "private",
    };
    let msg = format!("visibility set {thread_id} -> {label}");
    write_snapshot(git, thread_id, &doc, &msg)?;
    Ok(())
}
