use super::error::ForumResult;
use super::event::ThreadKind;
use super::git_ops::GitOps;
use super::policy::{self, GuardViolation, Policy};
use super::state_machine;
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

    Ok(VerifyReport {
        thread_id: thread_id.to_string(),
        violations,
        lookahead,
    })
}

/// The forward target state for preflight purposes (the acceptance-track terminal).
fn forward_target(kind: ThreadKind, status: &str) -> Option<&'static str> {
    match (kind, status) {
        (ThreadKind::Issue, "open") => Some("closed"),
        (ThreadKind::Rfc, "under-review") => Some("accepted"),
        (ThreadKind::Dec, "proposed") => Some("accepted"),
        (ThreadKind::Task, "reviewing") => Some("closed"),
        _ => None,
    }
}

/// The milestone target for each thread kind (the "happy path" endpoint).
fn milestone_target(kind: ThreadKind) -> &'static str {
    match kind {
        ThreadKind::Issue => "closed",
        ThreadKind::Rfc => "accepted",
        ThreadKind::Dec => "accepted",
        ThreadKind::Task => "closed",
    }
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
    let path = match state_machine::find_path(kind, current_status, target) {
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
