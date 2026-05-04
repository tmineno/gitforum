//! 1.x → 2.0 compatibility rules. See [`super`] for the full inventory.
//!
//! This module is the single read/load-time public API for the five
//! candidates the parent RFC (915yuegd P1) carved out. Domain code calls
//! these functions instead of duplicating the rule bodies inline; tests
//! at the bottom of this module pin every rule.

use super::super::event::{Event, Lifecycle, NodeType, ThreadKind};
use super::super::workflow::SPEC;

// =============================================================================
// 1. State alias normalisation
// =============================================================================

/// SPEC-2.0 §3.1.2 — pure text-level fold of 1.x state synonyms onto
/// their 2.0 canonical names.
///
/// `designing` and `implementing` both fold to `working`; this is lossy
/// on the 1.x→2.0 direction and intentional per the spec. `withdrawn`
/// passes through (it's a 2.0-valid state name); kind-aware adjustments
/// for execution/record threads live in [`migrate_legacy_state`].
const STATE_ALIASES: &[(&str, &str)] = &[
    ("proposed", "open"),
    ("under-review", "review"),
    ("reviewing", "review"),
    ("accepted", "done"),
    ("closed", "done"),
    ("pending", "working"),
    ("designing", "working"),
    ("implementing", "working"),
];

/// SPEC-2.0 §3.1.2 — fold a 1.x state synonym onto its 2.0 canonical name.
///
/// Returns `s` unchanged if it is not in the alias table. The lifetime
/// borrows from the input so a 2.0-already-canonical input is returned
/// without allocation.
pub fn normalize_state_name(s: &str) -> &str {
    STATE_ALIASES
        .iter()
        .find_map(|&(legacy, canonical)| (legacy == s).then_some(canonical))
        .unwrap_or(s)
}

/// SPEC-2.0 §3.1.1 / §3.1.2 — kind-aware migration of a 1.x state name
/// to a 2.0 state in the lifecycle's allowed set.
///
/// Composes [`normalize_state_name`] with one further per-lifecycle
/// trim: execution/record lifecycles do not allow `withdrawn`, so legacy
/// `withdrawn` Issue/Task/Dec threads remap to `rejected` (closest 2.0
/// semantic — work was abandoned without being deprecated).
pub fn migrate_legacy_state(kind: ThreadKind, state: &str) -> &str {
    let normalized = normalize_state_name(state);
    if normalized == "withdrawn" && !SPEC.allows_state(SPEC.kind_lifecycle(kind), "withdrawn") {
        "rejected"
    } else {
        normalized
    }
}

// =============================================================================
// 2. 1.x ThreadKind → lifecycle auto-derive (replay fallback)
// =============================================================================

/// SPEC-2.0 §2.3.3 — canonical lifecycle facet for a 1.x `ThreadKind`.
///
/// Used to derive `lifecycle` for legacy threads with no `facet_set`
/// event in their chain. Sources from the 2.0 kind-preset table; the
/// compat aspect is the *fallback role*, not the table itself.
pub fn lifecycle_for_legacy_kind(kind: ThreadKind) -> Lifecycle {
    SPEC.kind_lifecycle(kind)
}

// =============================================================================
// 3. NodeType canonicalisation
// =============================================================================

/// SPEC-2.0 §2.5 / §10.1 — project any `NodeType` to its 2.0 canonical
/// form.
///
/// - The seven 1.x prose-only variants collapse to `Comment`.
/// - `Evidence` collapses to `Comment` (the evidence-pointer surface
///   moves out of the node namespace entirely; see `evidence add`).
/// - `Comment`, `Approval`, `Objection`, `Action` are unchanged.
pub fn canonical_node_type(nt: NodeType) -> NodeType {
    match nt {
        NodeType::Comment | NodeType::Approval | NodeType::Objection | NodeType::Action => nt,
        NodeType::Claim
        | NodeType::Question
        | NodeType::Evidence
        | NodeType::Summary
        | NodeType::Risk
        | NodeType::Review
        | NodeType::Alternative
        | NodeType::Assumption => NodeType::Comment,
    }
}

/// `true` iff `nt` is one of the four 2.0 canonical variants
/// (`Comment`, `Approval`, `Objection`, `Action`).
pub fn is_canonical_node_type(nt: NodeType) -> bool {
    matches!(
        nt,
        NodeType::Comment | NodeType::Approval | NodeType::Objection | NodeType::Action
    )
}

// =============================================================================
// 5. Event.legacy_subtype preservation
// =============================================================================

/// 1.x rhetorical-subtype label for a non-canonical `NodeType`, or
/// `None` if `nt` is already canonical.
///
/// Used by 2.0 write paths and the migration tool to record the user's
/// stated rhetorical type in `Event.legacy_subtype` while persisting
/// the canonical [`canonical_node_type`] on the event itself
/// (SPEC-2.0 §2.5 / §9.3 / §10.1).
pub fn legacy_subtype_label(nt: NodeType) -> Option<&'static str> {
    match nt {
        NodeType::Comment | NodeType::Approval | NodeType::Objection | NodeType::Action => None,
        NodeType::Claim => Some("claim"),
        NodeType::Question => Some("question"),
        NodeType::Evidence => Some("evidence"),
        NodeType::Summary => Some("summary"),
        NodeType::Risk => Some("risk"),
        NodeType::Review => Some("review"),
        NodeType::Alternative => Some("alternative"),
        NodeType::Assumption => Some("assumption"),
    }
}

/// Owned copy of [`legacy_subtype_label`] for callers that need a
/// `String` (e.g. event field assignment via `Event.legacy_subtype`).
pub fn legacy_subtype_for_node_type(nt: NodeType) -> Option<String> {
    legacy_subtype_label(nt).map(str::to_string)
}

/// SPEC-2.0 §2.5 / §9.3 — apply the "persist canonical, preserve
/// rhetorical label" rule to an event builder.
///
/// Sets `node_type` to [`canonical_node_type`] and, when the original
/// is a 1.x non-canonical variant, records [`legacy_subtype_label`] on
/// `legacy_subtype`. Used by every native 2.0 write path that emits a
/// node-bearing event (`say`, `retype`, etc.) so the rule lives in one
/// place rather than being copy-pasted at each write site.
///
/// Read paths (replay, projection) use [`canonical_node_type`] and
/// [`legacy_subtype_label`] directly because they project an existing
/// stored event rather than build a new one.
pub fn apply_canonical_node_type(ev: Event, nt: NodeType) -> Event {
    let mut ev = ev.with_node_type(canonical_node_type(nt));
    if let Some(label) = legacy_subtype_label(nt) {
        ev = ev.with_legacy_subtype(label);
    }
    ev
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_known_aliases() {
        assert_eq!(normalize_state_name("accepted"), "done");
        assert_eq!(normalize_state_name("closed"), "done");
        assert_eq!(normalize_state_name("under-review"), "review");
        assert_eq!(normalize_state_name("reviewing"), "review");
        assert_eq!(normalize_state_name("proposed"), "open");
        assert_eq!(normalize_state_name("pending"), "working");
        assert_eq!(normalize_state_name("designing"), "working");
        assert_eq!(normalize_state_name("implementing"), "working");
    }

    #[test]
    fn normalize_passes_through_canonical() {
        for s in &[
            "draft",
            "open",
            "working",
            "review",
            "done",
            "rejected",
            "withdrawn",
            "deprecated",
        ] {
            assert_eq!(normalize_state_name(s), *s);
        }
    }

    #[test]
    fn migrate_remaps_withdrawn_for_execution_and_record() {
        // Proposal threads keep `withdrawn`.
        assert_eq!(
            migrate_legacy_state(ThreadKind::Rfc, "withdrawn"),
            "withdrawn"
        );
        // Execution / Record do not allow `withdrawn` -> remap to rejected.
        assert_eq!(
            migrate_legacy_state(ThreadKind::Issue, "withdrawn"),
            "rejected"
        );
        assert_eq!(
            migrate_legacy_state(ThreadKind::Task, "withdrawn"),
            "rejected"
        );
        assert_eq!(
            migrate_legacy_state(ThreadKind::Dec, "withdrawn"),
            "rejected"
        );
    }

    #[test]
    fn lifecycle_for_legacy_kind_matches_kind_presets() {
        assert_eq!(
            lifecycle_for_legacy_kind(ThreadKind::Rfc),
            Lifecycle::Proposal
        );
        assert_eq!(
            lifecycle_for_legacy_kind(ThreadKind::Issue),
            Lifecycle::Execution
        );
        assert_eq!(
            lifecycle_for_legacy_kind(ThreadKind::Task),
            Lifecycle::Execution
        );
        assert_eq!(
            lifecycle_for_legacy_kind(ThreadKind::Dec),
            Lifecycle::Record
        );
    }

    // legacy_kind_prefix_to_lifecycle test removed: the helper is no
    // longer used (3.0 policy parser rejects scope-string guards
    // entirely instead of translating kind prefixes).

    #[test]
    fn canonical_collapses_legacy_to_comment() {
        for nt in &[
            NodeType::Comment,
            NodeType::Approval,
            NodeType::Objection,
            NodeType::Action,
        ] {
            assert_eq!(canonical_node_type(*nt), *nt);
            assert!(is_canonical_node_type(*nt));
        }
        for nt in &[
            NodeType::Claim,
            NodeType::Question,
            NodeType::Evidence,
            NodeType::Summary,
            NodeType::Risk,
            NodeType::Review,
            NodeType::Alternative,
            NodeType::Assumption,
        ] {
            assert_eq!(canonical_node_type(*nt), NodeType::Comment);
            assert!(!is_canonical_node_type(*nt));
        }
    }

    #[test]
    fn legacy_subtype_label_round_trip() {
        assert_eq!(legacy_subtype_label(NodeType::Comment), None);
        assert_eq!(legacy_subtype_label(NodeType::Approval), None);
        assert_eq!(legacy_subtype_label(NodeType::Objection), None);
        assert_eq!(legacy_subtype_label(NodeType::Action), None);
        assert_eq!(legacy_subtype_label(NodeType::Claim), Some("claim"));
        assert_eq!(legacy_subtype_label(NodeType::Question), Some("question"));
        assert_eq!(legacy_subtype_label(NodeType::Evidence), Some("evidence"));
        assert_eq!(legacy_subtype_label(NodeType::Summary), Some("summary"));
        assert_eq!(legacy_subtype_label(NodeType::Risk), Some("risk"));
        assert_eq!(legacy_subtype_label(NodeType::Review), Some("review"));
        assert_eq!(
            legacy_subtype_label(NodeType::Alternative),
            Some("alternative")
        );
        assert_eq!(
            legacy_subtype_label(NodeType::Assumption),
            Some("assumption")
        );
    }
}
