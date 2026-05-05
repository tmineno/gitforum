//! `git forum state bulk` orchestration: bulk state-change with selectors
//! (`--branch` / `--kind` / `--status`) and per-thread report.
//!
//! Phase 2 slot 3 (RFC `7ymtc4b2`): state changes go through
//! `commands::state::apply_state_change_snapshot`, which writes
//! `thread.toml.status` directly. The legacy
//! `state_change::change_state` event-write path is no longer
//! invoked from the bulk arm.

use super::context::Context;
use super::shared::resolve_actor;
use crate::internal::clock::Clock;
use crate::internal::error::ForumError;
use crate::internal::git_ops::GitOps;
use crate::internal::policy::{self, Policy};
use crate::internal::refs;
use crate::internal::thread;

/// Args for `commands::bulk::run` — `state bulk` selector + transition.
pub struct BulkArgs {
    pub thread_ids: Vec<String>,
    pub branch: Option<String>,
    /// Canonical preset name (`"rfc"` / `"dec"` / `"task"` /
    /// `"issue"`) per `policy::kind_label_for`. v3.1 step 3n
    /// (task `1v400j3l`) replaced the typed `ThreadKind` filter.
    pub kind: Option<&'static str>,
    pub status: Option<String>,
    pub new_state: String,
    pub approve: Vec<String>,
    pub as_actor: Option<String>,
    pub resolve_open_actions: bool,
    pub dry_run: bool,
}

/// Uniform entry point per task `t8o3vnt6`.
pub fn run(args: BulkArgs, ctx: &Context) -> Result<(), ForumError> {
    let policy = Policy::load(&ctx.paths.dot_forum.join("policy.toml"))?;
    let actor = resolve_actor(args.as_actor, &ctx.git);
    let report = run_bulk_state_change(
        &ctx.git,
        &policy,
        &args.thread_ids,
        BulkSelectors {
            branch: args.branch.as_deref(),
            kind: args.kind,
            status: args.status.as_deref(),
        },
        &args.new_state,
        &args.approve,
        &actor,
        ctx.clock.as_ref(),
        args.resolve_open_actions,
        args.dry_run,
    )?;
    print_bulk_report(&report);
    if report.failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub struct BulkSelectors<'a> {
    pub branch: Option<&'a str>,
    pub kind: Option<&'static str>,
    pub status: Option<&'a str>,
}

pub struct BulkStateOutcome {
    pub thread_id: String,
    pub from_state: String,
    pub to_state: String,
    pub ok: bool,
    pub dry_run: bool,
    pub detail: Option<String>,
}

pub struct BulkStateReport {
    pub outcomes: Vec<BulkStateOutcome>,
    pub failures: usize,
}

/// Replay every thread (or every thread in `kind`/`branch` filter) and
/// return the materialised states sorted by creation time.
pub fn list_thread_states(
    git: &GitOps,
    kind: Option<&'static str>,
    branch: Option<&str>,
) -> Result<Vec<thread::ThreadState>, ForumError> {
    let all_ids = thread::list_thread_ids(git)?;
    let mut states = Vec::new();
    for id in &all_ids {
        match thread::replay_thread(git, id) {
            Ok(state) => {
                if thread_matches_filters(&state, kind, branch, None) {
                    states.push(state);
                }
            }
            Err(e) => {
                let ref_name = refs::thread_ref(id);
                eprintln!(
                    "warning: skipping {id}: failed to replay {ref_name}: {e}\n  \
                     hint: run `git forum doctor` to diagnose, \
                     or `git forum repair` to attempt recovery"
                );
            }
        }
    }
    states.sort_by_key(|s| s.created_at);
    Ok(states)
}

/// Predicate for kind AND branch AND status filters. Each filter is
/// `None` to skip; non-`None` matches the field. Kind is matched by
/// computing `policy::kind_label_for(category, tags)` (the canonical
/// preset name) since v3.1 step 3n removed `state.kind`.
pub fn thread_matches_filters(
    state: &thread::ThreadState,
    kind: Option<&'static str>,
    branch: Option<&str>,
    status: Option<&str>,
) -> bool {
    kind.is_none_or(|kind| policy::kind_label_for(&state.category, &state.tags) == kind)
        && branch.is_none_or(|branch| state.branch.as_deref() == Some(branch))
        && status.is_none_or(|status| state.status.as_str() == status)
}

#[allow(clippy::too_many_arguments)]
pub fn run_bulk_state_change(
    git: &GitOps,
    policy: &Policy,
    explicit_ids: &[String],
    selectors: BulkSelectors<'_>,
    new_state: &str,
    approve: &[String],
    actor: &str,
    clock: &dyn Clock,
    resolve_open_actions: bool,
    dry_run: bool,
) -> Result<BulkStateReport, ForumError> {
    if explicit_ids.is_empty()
        && selectors.branch.is_none()
        && selectors.kind.is_none()
        && selectors.status.is_none()
    {
        return Err(ForumError::Config(
            "state bulk requires at least one THREAD_ID or selector (--branch/--kind/--status)"
                .into(),
        ));
    }

    let candidate_ids = if explicit_ids.is_empty() {
        thread::list_thread_ids(git)?
    } else {
        explicit_ids.to_vec()
    };

    let mut outcomes = Vec::new();
    for thread_id in candidate_ids {
        let state = match thread::replay_thread(git, &thread_id) {
            Ok(state) => state,
            Err(err) => {
                outcomes.push(BulkStateOutcome {
                    thread_id,
                    from_state: "?".into(),
                    to_state: new_state.to_string(),
                    ok: false,
                    dry_run,
                    detail: Some(err.to_string()),
                });
                continue;
            }
        };

        if !thread_matches_filters(&state, selectors.kind, selectors.branch, selectors.status) {
            continue;
        }

        // Self-loop fast-path: avoid touching the snapshot tip when
        // the thread is already in the target. Normalize both sides
        // so 1.x verbs collapse onto canonical 2.0 names first.
        let canonical_target = policy::canonical_status_lenient(new_state).unwrap_or(new_state);
        let canonical_current = policy::canonical_status_lenient(state.status.as_str())
            .unwrap_or(state.status.as_str());
        if canonical_current == canonical_target {
            outcomes.push(BulkStateOutcome {
                thread_id,
                from_state: state.status.to_string(),
                to_state: new_state.to_string(),
                ok: true,
                dry_run,
                detail: Some(format!(
                    "already in '{canonical_target}'; no transition recorded"
                )),
            });
            continue;
        }

        if dry_run {
            outcomes.push(BulkStateOutcome {
                thread_id,
                from_state: state.status.to_string(),
                to_state: new_state.to_string(),
                ok: true,
                dry_run,
                detail: None,
            });
            continue;
        }

        match super::state::apply_state_change_snapshot(
            git,
            policy,
            &thread_id,
            new_state,
            approve,
            actor,
            clock,
            resolve_open_actions,
        ) {
            Ok((from, to)) => outcomes.push(BulkStateOutcome {
                thread_id,
                from_state: from,
                to_state: to.to_string(),
                ok: true,
                dry_run,
                detail: None,
            }),
            Err(err) => outcomes.push(BulkStateOutcome {
                thread_id,
                from_state: state.status.to_string(),
                to_state: new_state.to_string(),
                ok: false,
                dry_run,
                detail: Some(err.to_string()),
            }),
        }
    }

    if outcomes.is_empty() {
        return Err(ForumError::Config(
            "state bulk matched no threads for the given selectors".into(),
        ));
    }

    let failures = outcomes.iter().filter(|o| !o.ok).count();
    Ok(BulkStateReport { outcomes, failures })
}

pub fn print_bulk_report(report: &BulkStateReport) {
    for outcome in &report.outcomes {
        let marker = match (outcome.dry_run, outcome.ok) {
            (false, true) => "OK",
            (false, false) => "FAIL",
            (true, true) => "WOULD-OK",
            (true, false) => "WOULD-FAIL",
        };
        match &outcome.detail {
            Some(detail) => println!(
                "{marker:<10} {:<12} {} -> {}  {}",
                outcome.thread_id, outcome.from_state, outcome.to_state, detail
            ),
            None => println!(
                "{marker:<10} {:<12} {} -> {}",
                outcome.thread_id, outcome.from_state, outcome.to_state
            ),
        }
    }
}
