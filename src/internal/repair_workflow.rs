//! Repair historical `InvalidTransition` violations via append-only
//! corrective events (#uu9wxn1d).
//!
//! Background: P0 #34ith16h introduced `StrictReplayIssue::InvalidTransition`,
//! which surfaced 19 retroactive findings on this repo's own forum data
//! when run via `git forum doctor --strict`. Lenient replay materialises
//! the final state correctly; the issue is purely audit-visible under
//! strict mode.
//!
//! Categories:
//! - **Category A (`draft → rejected` on proposal)**: 17 threads. Repair
//!   appends `state open` then `state rejected`, which walks the chain
//!   back onto the canonical `draft → open → rejected` path. Subsequent
//!   strict replays detect the corrective tail and suppress the issue
//!   via `suppress_self_healed_invalid_transitions` in `thread.rs`.
//! - **Category B (`review → withdrawn` on proposal)**: 2 threads.
//!   Cannot be repaired via append-only — `withdrawn` has no legal
//!   outgoing edges in the proposal lifecycle, so no corrective walk
//!   exists. These threads are reported in the dry-run plan as
//!   `Unfixable` and remain flagged by doctor after `--apply`.
//!
//! Idempotency: a chain whose tail already self-heals is reported as
//! `AlreadyRepaired` and emits zero events on `--apply`.

use super::clock::Clock;
use super::error::ForumResult;
use super::event::{self, Event, EventType, Lifecycle};
use super::git_ops::GitOps;
use super::thread;
use super::validate::StrictReplayIssue;
use super::workflow::SPEC;

/// One thread's repair status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairAction {
    /// Append the listed `state` events (in order) to bring the chain
    /// onto a legal terminal path.
    Append { steps: Vec<&'static str> },
    /// Chain already self-heals; nothing to do.
    AlreadyRepaired,
    /// No append-only repair exists (Category B: terminal sink state).
    /// The InvalidTransition issue will remain after this run.
    Unfixable { reason: &'static str },
}

#[derive(Debug, Clone)]
pub struct RepairPlan {
    pub thread_id: String,
    pub category: RepairCategory,
    pub action: RepairAction,
    pub current_terminal: String,
    pub offending_event_id: String,
    pub illegal_from: String,
    pub illegal_to: String,
    pub lifecycle: Lifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairCategory {
    /// Illegal `draft → rejected` (or any other illegal edge that lands on
    /// a state with legal outgoing edges back to itself).
    SelfHealable,
    /// Illegal edge landing on a sink state (no legal outgoing edges).
    SinkTerminal,
}

#[derive(Debug, Default)]
pub struct RepairReport {
    pub plans: Vec<RepairPlan>,
    pub events_written: usize,
}

impl RepairReport {
    pub fn affected(&self) -> usize {
        self.plans
            .iter()
            .filter(|p| matches!(p.action, RepairAction::Append { .. }))
            .count()
    }

    pub fn unfixable(&self) -> usize {
        self.plans
            .iter()
            .filter(|p| matches!(p.action, RepairAction::Unfixable { .. }))
            .count()
    }

    pub fn already_repaired(&self) -> usize {
        self.plans
            .iter()
            .filter(|p| matches!(p.action, RepairAction::AlreadyRepaired))
            .count()
    }
}

/// Walk every thread, run strict replay, and produce a per-thread repair
/// plan for any chain carrying an `InvalidTransition` issue. Threads with
/// no such issue are not in the returned vector.
///
/// The returned plan distinguishes:
/// - `Append` — the chain still surfaces the issue under the public
///   suppressed-replay view; the corrective tail must be written.
/// - `AlreadyRepaired` — the chain self-heals (suppressed view is clean,
///   raw view still records the historical event); no events to write.
/// - `Unfixable` — the chain ends on a sink state with no legal
///   outgoing edges (Category B).
pub fn plan(git: &GitOps) -> ForumResult<Vec<RepairPlan>> {
    let thread_ids = thread::list_thread_ids(git)?;
    let mut plans = Vec::new();
    for tid in &thread_ids {
        let Ok(events) = event::load_thread_events(git, tid) else {
            continue;
        };
        let (state, suppressed_issues) = thread::replay_strict(&events)?;
        if let Some((event_id, from, to, lifecycle)) =
            first_invalid_transition(&suppressed_issues, state.lifecycle)
        {
            plans.push(build_plan(state, tid, event_id, from, to, lifecycle));
            continue;
        }
        // No InvalidTransition under the suppressed view. Did the chain
        // ever have one? Probe the raw view: if yes, the chain is
        // self-healed (likely from a prior repair run).
        let (raw_state, raw_issues) = thread::replay_strict_unsuppressed(&events)?;
        if let Some((event_id, from, to, lifecycle)) =
            first_invalid_transition(&raw_issues, raw_state.lifecycle)
        {
            plans.push(RepairPlan {
                thread_id: tid.clone(),
                category: RepairCategory::SelfHealable,
                action: RepairAction::AlreadyRepaired,
                current_terminal: raw_state.status.as_str().to_string(),
                offending_event_id: event_id,
                illegal_from: from,
                illegal_to: to,
                lifecycle,
            });
        }
    }
    Ok(plans)
}

fn first_invalid_transition(
    issues: &[StrictReplayIssue],
    fallback_lifecycle: Lifecycle,
) -> Option<(String, String, String, Lifecycle)> {
    issues.iter().find_map(|i| match i {
        StrictReplayIssue::InvalidTransition {
            event_id,
            from,
            to,
            lifecycle,
        } => Some((
            event_id.clone(),
            from.clone(),
            to.clone(),
            Lifecycle::parse(lifecycle).unwrap_or(fallback_lifecycle),
        )),
        _ => None,
    })
}

fn build_plan(
    state: thread::ThreadState,
    thread_id: &str,
    event_id: String,
    from: String,
    to: String,
    lifecycle: Lifecycle,
) -> RepairPlan {
    let terminal = state.status.as_str().to_string();
    // Decide if a legal corrective walk exists: the running state at the
    // chain's tail must have at least one legal outgoing edge that
    // ultimately returns to `terminal`.
    let has_outgoing = !SPEC.valid_targets(lifecycle, &terminal).is_empty();
    if !has_outgoing {
        return RepairPlan {
            thread_id: thread_id.to_string(),
            category: RepairCategory::SinkTerminal,
            action: RepairAction::Unfixable {
                reason: "terminal status has no legal outgoing edges in this lifecycle (append-only repair impossible)",
            },
            current_terminal: terminal,
            offending_event_id: event_id,
            illegal_from: from,
            illegal_to: to,
            lifecycle,
        };
    }
    let steps = corrective_steps(&terminal, lifecycle);
    let action = if steps.is_empty() {
        RepairAction::Unfixable {
            reason: "no two-step corrective walk found from terminal status",
        }
    } else {
        RepairAction::Append { steps }
    };
    RepairPlan {
        thread_id: thread_id.to_string(),
        category: RepairCategory::SelfHealable,
        action,
        current_terminal: terminal,
        offending_event_id: event_id,
        illegal_from: from,
        illegal_to: to,
        lifecycle,
    }
}

/// Pick a two-step corrective walk: terminal → intermediate → terminal,
/// where both edges are legal under the lifecycle. Returns the
/// intermediate + the final terminal as the steps to emit; the caller
/// appends a `state <step>` event for each.
///
/// Today's data hits exactly one shape — `rejected` terminal with the
/// `rejected → open → rejected` walk — but the search is generic so a
/// future violation that matches a different terminal (e.g. `done`)
/// would also resolve.
fn corrective_steps(terminal: &str, lifecycle: Lifecycle) -> Vec<&'static str> {
    // valid_targets returns &'static str values (the workflow data tables
    // are static), so we can return them directly.
    for intermediate in SPEC.valid_targets(lifecycle, terminal) {
        if intermediate == terminal {
            continue;
        }
        if SPEC.is_valid_transition(lifecycle, intermediate, terminal) {
            // Look up the static-str form of `terminal` from the
            // allowed-states table so the final element is also &'static.
            if let Some(canonical_terminal) = SPEC
                .allowed_states(lifecycle)
                .iter()
                .copied()
                .find(|s| *s == terminal)
            {
                return vec![intermediate, canonical_terminal];
            }
        }
    }
    Vec::new()
}

/// Apply each plan's `Append` action by writing `state` events to git.
/// Returns the total number of events written.
///
/// `actor` is recorded as the event's actor; not back-dated. Callers
/// supply the running clock so timestamps reflect when the repair ran.
pub fn apply(
    git: &GitOps,
    plans: &[RepairPlan],
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<usize> {
    let mut written = 0usize;
    for plan in plans {
        let RepairAction::Append { steps } = &plan.action else {
            continue;
        };
        for step in steps {
            let ev = Event::base(&plan.thread_id, EventType::State, actor, clock)
                .with_new_state(step)
                .with_body(&format!(
                    "Corrective state event for #uu9wxn1d (repair of {} '{}' → '{}' for {} lifecycle).",
                    plan.offending_event_id, plan.illegal_from, plan.illegal_to, plan.lifecycle,
                ));
            event::write_event(git, &ev)?;
            written += 1;
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::clock::FixedClock;
    use chrono::TimeZone;

    fn fixed_clock() -> FixedClock {
        FixedClock {
            instant: chrono::Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn category_a_corrective_steps_resolve_to_open_rejected() {
        // Proposal lifecycle, terminal = rejected → corrective walk is
        // rejected → open → rejected (the only two-step legal cycle
        // back to rejected).
        let steps = corrective_steps("rejected", Lifecycle::Proposal);
        assert_eq!(steps, vec!["open", "rejected"]);
    }

    #[test]
    fn sink_terminal_has_no_corrective_steps() {
        // Withdrawn is a sink in proposal lifecycle.
        let steps = corrective_steps("withdrawn", Lifecycle::Proposal);
        assert!(steps.is_empty(), "sink should have no corrective walk");
    }

    #[test]
    fn corrective_walk_is_legal_for_every_lifecycle_with_outgoing_edges() {
        // For each (lifecycle, allowed terminal), if it has outgoing edges
        // the proposed corrective steps must form a legal cycle.
        for lc in [Lifecycle::Proposal, Lifecycle::Execution, Lifecycle::Record] {
            for state in SPEC.allowed_states(lc) {
                let steps = corrective_steps(state, lc);
                if steps.is_empty() {
                    continue;
                }
                assert_eq!(steps.len(), 2, "expected 2 steps");
                assert!(
                    SPEC.is_valid_transition(lc, state, steps[0]),
                    "{lc}: {state} -> {} should be legal",
                    steps[0]
                );
                assert!(
                    SPEC.is_valid_transition(lc, steps[0], steps[1]),
                    "{lc}: {} -> {} should be legal",
                    steps[0],
                    steps[1]
                );
                assert_eq!(steps[1], *state, "walk must return to terminal");
            }
        }
    }

    #[test]
    fn fixed_clock_compiles() {
        let _ = fixed_clock();
    }
}
