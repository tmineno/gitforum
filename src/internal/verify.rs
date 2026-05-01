use super::error::ForumResult;
use super::event::ThreadKind;
use super::event::{self, normalize_state_name};
use super::git_ops::GitOps;
use super::policy::{self, GuardViolation, Policy};
use super::thread;

/// Result of a preflight check (`git forum verify`).
///
/// This is a forward-transition readiness check, not a history audit.
/// It evaluates policy guards for the thread's next expected transition
/// (e.g. `open->closed` for issues, `under-review->accepted` for RFCs).
///
/// When the thread is not yet at the direct pre-target state, `lookahead`
/// shows guard violations that would block the eventual forward target,
/// so the user can plan ahead.
#[derive(Debug)]
pub struct VerifyReport {
    pub thread_id: String,
    pub violations: Vec<GuardViolation>,
    /// Guard violations for milestone states reachable via intermediate transitions.
    /// Each entry is (from_state, to_state, path_description, violations).
    pub lookahead: Vec<LookaheadEntry>,
    /// SPEC-2.0 §9.4 advisory: informational notes about the state of threads
    /// linked from the verified thread. Strictly informational — the
    /// verification result above is computed only from the named thread.
    pub linked_advisories: Vec<LinkedAdvisory>,
}

#[derive(Debug)]
pub struct LookaheadEntry {
    /// The transition being checked (e.g. "under-review -> accepted")
    pub transition: String,
    /// The path from the current state (e.g. "proposed → under-review → accepted")
    pub path: String,
    /// Guard violations that would block this transition
    pub violations: Vec<GuardViolation>,
}

/// Advisory line about a thread linked from the verified thread.
///
/// Informational only (per CORE-VALUE.md "Advisories"). Generated only when
/// the linked thread's state is observably "not yet done" — to surface the
/// likely reader question without ever blocking the verification.
#[derive(Debug, Clone)]
pub struct LinkedAdvisory {
    pub linked_thread_id: String,
    pub linked_kind: ThreadKind,
    pub linked_status: String,
    pub rel: String,
    pub message: String,
}

impl VerifyReport {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Preflight check: evaluate policy guards for the thread's next forward transition.
///
/// Preconditions: thread_id exists; policy is loaded.
/// Postconditions: returns VerifyReport with blocking guard violations (empty = ready).
/// Failure modes: ForumError::Git on replay failure.
/// Side effects: none (read-only).
pub fn verify_thread(git: &GitOps, thread_id: &str, p: &Policy) -> ForumResult<VerifyReport> {
    let state = thread::replay_thread(git, thread_id)?;
    let violations = match forward_target(state.kind, &state.status) {
        Some(to) => policy::check_guards(p, &state, &state.status, to),
        None => vec![],
    };

    // Lookahead: check guards for milestone states reachable via intermediate transitions
    let lookahead = build_lookahead(state.kind, &state.status, &state, p);

    // Advisory: surface state of linked threads (strictly informational).
    let linked_advisories = build_linked_advisories(git, &state);

    Ok(VerifyReport {
        thread_id: thread_id.to_string(),
        violations,
        lookahead,
        linked_advisories,
    })
}

/// Walk the verified thread's forward links one hop, replay each linked
/// thread's tip ref, and emit an advisory if it isn't yet `done`.
///
/// This is intentionally read-only and best-effort: a missing or unreplayable
/// linked thread is silently skipped, not surfaced as a verify error. Link
/// target IDs that pre-date the Track C migration are recorded in the legacy
/// `KIND-XXXXXXXX` form; we resolve those to canonical bare-token form before
/// replay so the advisory works on migrated repos.
///
/// The guard preflight result (above) is the verification's authoritative
/// answer; these lines exist only to make the cross-thread context visible
/// without gating anything.
fn build_linked_advisories(git: &GitOps, state: &thread::ThreadState) -> Vec<LinkedAdvisory> {
    let mut out = Vec::new();
    for link in &state.links {
        let canonical = thread::resolve_thread_id(git, &link.target_thread_id)
            .unwrap_or_else(|_| link.target_thread_id.clone());
        let Ok(linked) = thread::replay_thread(git, &canonical) else {
            continue;
        };
        if normalize_state_name(&linked.status) == "done" {
            continue;
        }
        out.push(LinkedAdvisory {
            linked_thread_id: linked.id.clone(),
            linked_kind: linked.kind,
            linked_status: linked.status.clone(),
            rel: link.rel.clone(),
            message: format!(
                "linked {} {} ({}) is not yet `done` — informational only",
                linked.kind, linked.id, linked.status
            ),
        });
    }
    out
}

/// The forward target state for preflight purposes (the milestone terminal,
/// always `done` post-2.0). Returned only when the current state has a
/// direct guarded edge to the milestone for this kind.
fn forward_target(kind: ThreadKind, status: &str) -> Option<&'static str> {
    let normalized = normalize_state_name(status);
    match (kind, normalized) {
        // Execution: open → done is the typical guarded close for bugs.
        (ThreadKind::Issue, "open") => Some("done"),
        // Proposal: review → done is the acceptance step.
        (ThreadKind::Rfc, "review") => Some("done"),
        // Record: open → done.
        (ThreadKind::Dec, "open") => Some("done"),
        // Execution (task): review → done is the acceptance step.
        (ThreadKind::Task, "review") => Some("done"),
        _ => None,
    }
}

/// The milestone target for each thread kind (the "happy path" endpoint).
/// Post-2.0 every kind's milestone is `done`.
fn milestone_target(_kind: ThreadKind) -> &'static str {
    "done"
}

/// Build lookahead entries for guards on milestone states reachable via
/// intermediate transitions. Only produces entries when the milestone target
/// is NOT a direct transition from the current state (i.e. when forward_target
/// returns None) and when a path exists.
pub fn build_lookahead(
    kind: ThreadKind,
    current_status: &str,
    state: &thread::ThreadState,
    policy: &Policy,
) -> Vec<LookaheadEntry> {
    // If forward_target already covers this state, no lookahead needed
    if forward_target(kind, current_status).is_some() {
        return vec![];
    }

    let target = milestone_target(kind);

    // Find the path from current state to the milestone target
    let path = match event::find_path(kind.lifecycle(), current_status, target) {
        Some(p) if p.len() >= 2 => p, // Need at least 2 steps (otherwise it's direct)
        _ => return vec![],
    };

    // The guard check is on the final transition: penultimate -> target
    let from_state = path[path.len() - 2];
    let violations = policy::check_guards(policy, state, from_state, target);

    if violations.is_empty() {
        return vec![];
    }

    let mut path_parts = vec![current_status.to_string()];
    for step in &path {
        path_parts.push(step.to_string());
    }

    vec![LookaheadEntry {
        transition: format!("{from_state} -> {target}"),
        path: path_parts.join(" → "),
        violations,
    }]
}
