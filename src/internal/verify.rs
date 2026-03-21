use super::error::ForumResult;
use super::event::ThreadKind;
use super::git_ops::GitOps;
use super::policy::{self, GuardViolation, Policy};
use super::thread;

/// Result of a `git forum verify` run.
#[derive(Debug)]
pub struct VerifyReport {
    pub thread_id: String,
    pub violations: Vec<GuardViolation>,
}

impl VerifyReport {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Verify a thread against all policy guards for its next expected transition.
///
/// Preconditions: thread_id exists; policy is loaded.
/// Postconditions: returns VerifyReport with all guard violations.
/// Failure modes: ForumError::Git on replay failure.
/// Side effects: none (read-only).
pub fn verify_thread(git: &GitOps, thread_id: &str, p: &Policy) -> ForumResult<VerifyReport> {
    let state = thread::replay_thread(git, thread_id)?;
    let violations = match forward_target(state.kind, &state.status) {
        Some(to) => policy::check_guards(p, &state, &state.status, to, &[]),
        None => vec![],
    };
    Ok(VerifyReport {
        thread_id: thread_id.to_string(),
        violations,
    })
}

/// The "forward" target state for verify purposes (the acceptance-track terminal).
fn forward_target(kind: ThreadKind, status: &str) -> Option<&'static str> {
    match (kind, status) {
        (ThreadKind::Issue, "open") => Some("closed"),
        (ThreadKind::Rfc, "under-review") => Some("accepted"),
        (ThreadKind::Dec, "proposed") => Some("accepted"),
        (ThreadKind::Task, "reviewing") => Some("closed"),
        _ => None,
    }
}
