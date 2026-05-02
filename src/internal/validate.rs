//! Strict replay validation (SPEC-2.0 §B.6 / Track-D follow-up).
//!
//! [`thread::replay`] is **lenient**: it silently no-ops on conditions that
//! point to corruption or invariant violations, because read-side callers
//! (CLI display, TUI render, search) prefer best-effort over hard failure.
//!
//! Doctor, migration verification, and tests need the opposite: every silent
//! no-op MUST surface so the operator can fix the underlying event-store
//! damage. [`StrictReplayIssue`] is the audit channel that lenient replay
//! discards but [`thread::replay_strict`] returns alongside the materialized
//! state.
//!
//! Today's checks (Phase-1 scope):
//! - `Edit` / `Retract` / `Resolve` / `Reopen` / `Retype` targeting an unknown
//!   `target_node_id` (lenient: silently no-op; strict: reported).
//! - Required-field gaps per event type (lenient: silently no-op via `if let`;
//!   strict: reported).
//! - A second `facet_set` carrying `lifecycle` after the first one (SPEC-2.0
//!   §7.3 says write-side MUST reject with `FacetTransitionDisallowed`; replay
//!   stays first-wins but reports the attempted reset).
//! - `facet_set` carrying a `lifecycle` value outside `proposal | execution |
//!   record`.

use super::event::EventType;

/// A semantic issue detected by strict replay.
///
/// Each variant names the offending event and the rule it violated. The
/// lenient `replay()` path swallows these; doctor / migration / tests use
/// `replay_strict()` to surface them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrictReplayIssue {
    /// `Edit` / `Retract` / `Resolve` / `Reopen` / `Retype` referenced a node
    /// that does not exist in the replayed state.
    UnknownTargetNode {
        event_id: String,
        event_type: EventType,
        target_node_id: String,
    },
    /// An event lacked a field that its `event_type` requires.
    MissingRequiredField {
        event_id: String,
        event_type: EventType,
        field: &'static str,
    },
    /// SPEC-2.0 §7.3: a second `facet_set` carrying `lifecycle` was applied.
    /// Replay keeps the first-set value (immutable), but write-side should
    /// have rejected this event with `FacetTransitionDisallowed`.
    LifecycleResetAttempted {
        event_id: String,
        existing: String,
        attempted: String,
    },
    /// `facet_set` carried a `lifecycle` value outside the canonical set.
    InvalidLifecycleValue { event_id: String, value: String },
}

impl std::fmt::Display for StrictReplayIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTargetNode {
                event_id,
                event_type,
                target_node_id,
            } => write!(
                f,
                "{event_type} event {event_id} targets unknown node {target_node_id}"
            ),
            Self::MissingRequiredField {
                event_id,
                event_type,
                field,
            } => write!(
                f,
                "{event_type} event {event_id} is missing required field '{field}'"
            ),
            Self::LifecycleResetAttempted {
                event_id,
                existing,
                attempted,
            } => write!(
                f,
                "facet_set event {event_id} attempted to reset lifecycle from '{existing}' to '{attempted}' (SPEC-2.0 §7.3: lifecycle is immutable after creation)"
            ),
            Self::InvalidLifecycleValue { event_id, value } => write!(
                f,
                "facet_set event {event_id} carries invalid lifecycle '{value}' (allowed: proposal | execution | record)"
            ),
        }
    }
}
