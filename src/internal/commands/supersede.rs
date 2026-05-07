//! `git forum supersede <OLD> --by <NEW>` orchestration.
//!
//! Ticket `eda1c050`: collapses the three-step supersede recipe
//! (`link --rel superseded-by`, `comment`, terminal-state transition)
//! into a single verb that lands `<old>` in `deprecated`. Distinguishes
//! superseded threads from genuinely-rejected work in
//! `git forum ls --status rejected`.
//!
//! Symmetric link writeback: also appends a `supersedes` link to the
//! `<new>` snapshot so `git forum show <NEW>` displays the supersede
//! relationship without a reverse-link index (the SQLite reverse index
//! was deleted in task `913c4s9v`).

use chrono::Utc;

use super::context::Context;
use crate::internal::clock::Clock;
use crate::internal::error::ForumError;
use crate::internal::git_ops::GitOps;
use crate::internal::snapshot::{self, store::write_snapshot, Link};

use super::shared::{discover_repo_with_init_warning, resolve_actor, resolve_tid};
use super::state::{run_state_shorthand, StateShorthandArgs};

pub struct SupersedeArgs {
    pub thread_id: String,
    pub by: String,
    pub body: Option<String>,
    pub as_actor: Option<String>,
    pub fast_track: bool,
    pub force: bool,
}

pub fn run(args: SupersedeArgs, ctx: &Context) -> Result<(), ForumError> {
    let SupersedeArgs {
        thread_id,
        by,
        body,
        as_actor,
        fast_track,
        force,
    } = args;

    let (git, _paths) = discover_repo_with_init_warning()?;
    let old_id = resolve_tid(&git, &thread_id)?;
    let new_id = resolve_tid(&git, &by)?;
    if old_id == new_id {
        return Err(ForumError::Config(
            "supersede: <thread_id> and --by must refer to different threads".into(),
        ));
    }
    let actor = resolve_actor(as_actor.clone(), &git);
    let comment = body.unwrap_or_else(|| format!("Superseded by @{new_id}"));

    // Old side: superseded-by link + comment + state -> deprecated, all
    // bundled into the same snapshot commit by run_state_shorthand.
    let shorthand = StateShorthandArgs {
        thread_id: old_id.clone(),
        new_state: "deprecated".into(),
        approve: vec![],
        as_actor,
        resolve_open_actions: false,
        link_to: vec![new_id.clone()],
        rel: Some("superseded-by".into()),
        comment: Some(comment),
        fast_track,
        force,
    };
    run_state_shorthand(
        &shorthand.thread_id,
        &shorthand.new_state,
        &shorthand.approve,
        shorthand.as_actor,
        shorthand.resolve_open_actions,
        &shorthand.link_to,
        shorthand.rel.as_deref(),
        shorthand.comment.as_deref(),
        shorthand.fast_track,
        shorthand.force,
        ctx.clock.as_ref(),
    )?;

    // New side: write back the symmetric `supersedes` link so
    // `git forum show <NEW>` surfaces the supersede relationship.
    write_back_supersedes_link(&git, &new_id, &old_id, &actor, ctx.clock.as_ref())?;

    Ok(())
}

fn write_back_supersedes_link(
    git: &GitOps,
    new_id: &str,
    old_id: &str,
    actor: &str,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let mut new_doc = snapshot::read_snapshot(git, new_id)?;
    if new_doc
        .links
        .entries
        .iter()
        .any(|l| l.target == old_id && l.rel == "supersedes")
    {
        return Ok(());
    }
    let now: chrono::DateTime<Utc> = clock.now();
    new_doc.links.entries.push(Link {
        target: old_id.into(),
        rel: "supersedes".into(),
        created_at: now,
        created_by: actor.into(),
    });
    new_doc.snapshot.updated_at = now;
    new_doc.snapshot.updated_by = actor.into();
    write_snapshot(git, new_id, &new_doc, &format!("link supersedes {old_id}"))?;
    Ok(())
}
