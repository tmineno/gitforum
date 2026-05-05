//! `git forum verify <THREAD_ID>` orchestration + the `VerifyReport`
//! data model.
//!
//! Phase 2 slot 7i (RFC `7ymtc4b2`): the `Verify` arm body relocates
//! from `main.rs` to [`run`] in this module. The
//! [`remediation_hint`] formatter moves out of `internal::state_change`
//! into this module — `verify` is its only remaining caller after the
//! state-change body relocation in slot 3.

use super::super::error::{ForumError, ForumResult};
use super::super::event::{self, normalize_state_name};
use super::super::git_ops::GitOps;
use super::super::policy::{self, GuardViolation, Policy};
use super::super::thread;
use super::super::thread::ThreadKind;
use super::context::Context;
use super::shared::resolve_tid;

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
    let violations = match forward_target(&state, p) {
        Some(to) => policy::check_guards(p, &state, state.status.as_str(), &to),
        None => vec![],
    };

    // Lookahead: check guards for milestone states reachable via intermediate transitions
    let lookahead = build_lookahead(&state, p);

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
        if linked.status == event::ThreadStatus::Done {
            continue;
        }
        out.push(LinkedAdvisory {
            linked_thread_id: linked.id.clone(),
            linked_kind: linked.kind,
            linked_status: linked.status.to_string(),
            rel: link.rel.clone(),
            message: format!(
                "linked {} {} ({}) is not yet `done` — informational only",
                linked.kind, linked.id, linked.status
            ),
        });
    }
    out
}

/// The forward target state for preflight purposes — the next direct edge
/// from the thread's current status whose destination is `done` (the
/// milestone terminal). SPEC-3.0 §3.1: routed through the effective
/// category registry, so `[categories.X] transitions = [...]` overrides
/// in `.forum/policy.toml` are honoured. Returns `None` when no direct
/// edge to the milestone exists from the current status.
fn forward_target(state: &thread::ThreadState, p: &Policy) -> Option<String> {
    let category = policy::category_for_state(state);
    let registry = p.effective_registry();
    let cat_def = registry.get(category)?;
    let normalized = normalize_state_name(state.status.as_str());
    cat_def
        .valid_targets(normalized)
        .into_iter()
        .find(|t| t == "done")
}

/// The milestone target for the verified thread (the "happy path"
/// endpoint). SPEC-3.0 §3.1 collapses every category's milestone to
/// `done`.
fn milestone_target() -> &'static str {
    "done"
}

/// Build lookahead entries for guards on milestone states reachable via
/// intermediate transitions. Only produces entries when the milestone target
/// is NOT a direct transition from the current state (i.e. when forward_target
/// returns None) and when a path exists. SPEC-3.0 §3.1: pathfinding goes
/// through the effective category registry so policy overrides shape the
/// reachable transition graph.
pub fn build_lookahead(state: &thread::ThreadState, policy: &Policy) -> Vec<LookaheadEntry> {
    // If forward_target already covers this state, no lookahead needed.
    if forward_target(state, policy).is_some() {
        return vec![];
    }

    let target = milestone_target();
    let category = policy::category_for_state(state);
    let registry = policy.effective_registry();
    let Some(cat_def) = registry.get(category) else {
        return vec![];
    };
    let current_status = state.status.as_str();

    // Find the path from current state to the milestone target.
    let path = match cat_def.find_path(current_status, target) {
        Some(p) if p.len() >= 2 => p,
        _ => return vec![],
    };

    // The guard check is on the final transition: penultimate -> target.
    let from_state = path[path.len() - 2].clone();
    let violations = policy::check_guards(policy, state, &from_state, target);

    if violations.is_empty() {
        return vec![];
    }

    let mut path_parts = vec![current_status.to_string()];
    for step in &path {
        path_parts.push(step.clone());
    }

    vec![LookaheadEntry {
        transition: format!("{from_state} -> {target}"),
        path: path_parts.join(" → "),
        violations,
    }]
}

// ============================================================
//  Verify command — `git forum verify` orchestration
// ============================================================

/// Args for [`run`] — `git forum verify`.
pub struct VerifyArgs {
    pub thread_id: String,
}

/// Uniform entry point for the `verify` subcommand.
///
/// Replays the thread, computes the policy guard preflight via
/// [`verify_thread`], and emits the same plain-text report shape that
/// `main.rs` previously hand-rolled. Exits non-zero iff the report
/// failed the preflight.
pub fn run(args: VerifyArgs, ctx: &Context) -> Result<(), ForumError> {
    let thread_id = resolve_tid(&ctx.git, &args.thread_id)?;
    let policy = Policy::load(&ctx.paths.dot_forum.join("policy.toml"))?;
    let report = verify_thread(&ctx.git, &thread_id, &policy)?;
    if report.passed() {
        println!("{thread_id}: ready");
    } else {
        let state = thread::replay_thread(&ctx.git, &thread_id)?;
        println!("{thread_id}: not ready");
        for v in &report.violations {
            println!("  BLOCKED [{}] {}", v.rule, v.reason);
            let hint = remediation_hint(&v.rule, &state, &thread_id);
            if !hint.is_empty() {
                println!("    fix: {hint}");
            }
        }
    }
    for entry in &report.lookahead {
        println!("  lookahead ({}):", entry.path);
        for v in &entry.violations {
            println!("    [{}] {}", v.rule, v.reason);
        }
    }
    for adv in &report.linked_advisories {
        println!("  advisory: {}", adv.message);
    }
    if !report.passed() {
        std::process::exit(1);
    }
    Ok(())
}

/// Format a remediation hint for a guard rule violation.
///
/// Phase 2 slot 7i (RFC `7ymtc4b2`): relocated from
/// `internal::state_change`. `verify` is the only remaining caller
/// after slot 3 (`state` rewire) — the legacy state-change pre-walk
/// in `state_change.rs` was the other consumer, and it now imports
/// this back-pointer until Phase 4 deletes that module.
pub fn remediation_hint(rule: &str, state: &thread::ThreadState, thread_id: &str) -> String {
    match rule {
        "no_open_actions" => {
            let ids: Vec<String> = state
                .open_actions()
                .iter()
                .map(|n| n.node_id[..n.node_id.len().min(16)].to_string())
                .collect();
            if ids.is_empty() {
                return String::new();
            }
            format!(
                "resolve each with `resolve {thread_id} <NODE_ID>` (open: {}) or use --resolve-open-actions",
                ids.join(", ")
            )
        }
        "no_open_objections" => {
            let ids: Vec<String> = state
                .open_objections()
                .iter()
                .map(|n| n.node_id[..n.node_id.len().min(16)].to_string())
                .collect();
            if ids.is_empty() {
                return String::new();
            }
            format!(
                "resolve each with `resolve {thread_id} <NODE_ID>` (open: {})",
                ids.join(", ")
            )
        }
        // `at_least_one_summary` was removed in 2.0 (ADR-006); the rule
        // never fires after Policy::load strips it. Hint left empty.
        "one_human_approval" => "supply --approve human/<name>".to_string(),
        _ => String::new(),
    }
}
