//! `git forum shortlog --since <DATE>` orchestration.
//!
//! Phase 2 slot 7b (RFC `7ymtc4b2`): NEW module. The arm body
//! relocates from `main.rs` to [`run`] here, and `terminal_state_date`
//! moves out of `commands::shared` into [`terminal_state_date`] below
//! and grows a snapshot-tip code path: for snapshot-only threads
//! (no event chain to scan) the "reached terminal state at" timestamp
//! is the git commit timestamp of `refs/forum/threads/<id>` per
//! SPEC-3.0 §5.4.

use chrono::{DateTime, Utc};

use super::super::error::ForumError;
use super::super::git_ops::GitOps;
use super::super::policy::TERMINAL_STATES;
use super::super::refs;
use super::super::thread::ThreadState;
use super::context::Context;

/// Args for [`run`] — `git forum shortlog` filters.
pub struct ShortlogArgs {
    pub since: String,
    pub kind: Option<String>,
}

/// Uniform entry point for the `shortlog` subcommand.
///
/// Replays every thread (filtered by `--kind`), keeps those whose
/// terminal-state timestamp is at or after `--since`, and prints the
/// grouped renderer in `commands::ls::render_shortlog`.
pub fn run(args: ShortlogArgs, ctx: &Context) -> Result<(), ForumError> {
    let kind_filter = super::shared::parse_thread_kind_filter(args.kind.as_deref())?;
    let since_dt = super::shared::parse_since_date(&args.since, &ctx.git)?;
    let states = super::bulk::list_thread_states(&ctx.git, kind_filter, None)?;
    let mut entries: Vec<(&ThreadState, DateTime<Utc>)> = Vec::new();
    for state in &states {
        if let Some(term_date) = terminal_state_date(&ctx.git, state) {
            if term_date >= since_dt {
                entries.push((state, term_date));
            }
        }
    }
    print!("{}", super::ls::render_shortlog(&entries));
    Ok(())
}

/// Find the timestamp at which a thread reached its current terminal
/// state. Returns `None` when the thread is not currently terminal.
///
/// Phase 2 slot 7b: snapshot-only threads (no event chain) use the git
/// commit timestamp of `refs/forum/threads/<id>` as the terminal-state
/// date. Mixed-chain threads with a tail event still resolve via the
/// legacy `EventType::State` scan; pure-event-chain threads keep the
/// pre-cutover behaviour. Either path returns the same answer for
/// threads that are currently terminal.
pub fn terminal_state_date(git: &GitOps, state: &ThreadState) -> Option<DateTime<Utc>> {
    use crate::internal::event::EventType;

    if !TERMINAL_STATES.contains(&state.status.as_str()) {
        return None;
    }
    if let Some(date) = state.events.iter().rev().find_map(|e| {
        if e.event_type == EventType::State && e.new_state.as_deref() == Some(state.status.as_str())
        {
            Some(e.created_at)
        } else {
            None
        }
    }) {
        return Some(date);
    }
    // SPEC-3.0 §5.4: snapshot-only thread — use the git commit
    // timestamp of the thread ref tip as the terminal-state date.
    git.commit_timestamp(&refs::thread_ref(&state.id)).ok()
}
