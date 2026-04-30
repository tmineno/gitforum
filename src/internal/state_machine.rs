use std::collections::VecDeque;

use super::event::{Lifecycle, ThreadKind};

/// SPEC-2.0 §3.1 — single unified transition graph.
///
/// Every edge any lifecycle might need; `Lifecycle::allowed_states` (§3.1.1)
/// filters reachability per thread. State names are 2.0 canonical.
pub const UNIFIED_TRANSITIONS: &[(&str, &str)] = &[
    ("draft", "open"),
    ("draft", "withdrawn"),
    ("open", "working"),
    ("open", "review"),
    ("open", "done"),
    ("open", "rejected"),
    ("open", "withdrawn"),
    ("working", "review"),
    ("working", "done"),
    ("working", "rejected"),
    ("review", "done"),
    ("review", "working"),
    ("review", "rejected"),
    ("done", "open"),
    ("rejected", "open"),
    ("done", "deprecated"),
    ("rejected", "deprecated"),
];

/// SPEC-2.0 §3.1.2 — pure text-level normalization of 1.x state names to 2.0.
///
/// `designing` and `implementing` both fold to `working`; this is lossy on
/// the 1.x→2.0 direction and intentional per the spec. Withdrawn passes
/// through (it's a 2.0-valid state name); kind-aware adjustments for
/// withdrawn-execution / withdrawn-record live in [`migrate_legacy_state`].
pub fn normalize_state_name(s: &str) -> &str {
    match s {
        "proposed" => "open",
        "under-review" | "reviewing" => "review",
        "accepted" | "closed" => "done",
        "pending" | "designing" | "implementing" => "working",
        _ => s,
    }
}

/// SPEC-2.0 §3.1.1 / §3.1.2 — kind-aware migration of a 1.x state name to a
/// 2.0 state in the lifecycle's allowed set. Composes
/// [`normalize_state_name`] with one further per-lifecycle trim:
/// execution/record lifecycles do not allow `withdrawn`, so legacy
/// `withdrawn` Issue/Task/Dec threads remap to `rejected` (closest 2.0
/// semantic — work was abandoned without being deprecated).
pub fn migrate_legacy_state(kind: ThreadKind, state: &str) -> &str {
    let normalized = normalize_state_name(state);
    if normalized == "withdrawn" && !kind.lifecycle().allows_state("withdrawn") {
        "rejected"
    } else {
        normalized
    }
}

impl Lifecycle {
    /// SPEC-2.0 §3.1.1 — initial state per lifecycle.
    pub fn initial_state(self) -> &'static str {
        match self {
            Self::Proposal => "draft",
            Self::Execution | Self::Record => "open",
        }
    }

    /// SPEC-2.0 §3.1.1 — states reachable for this lifecycle.
    pub fn allowed_states(self) -> &'static [&'static str] {
        match self {
            Self::Proposal => &[
                "draft",
                "open",
                "review",
                "done",
                "rejected",
                "withdrawn",
                "deprecated",
            ],
            Self::Execution => &[
                "open",
                "working",
                "review",
                "done",
                "rejected",
                "deprecated",
            ],
            Self::Record => &["open", "done", "rejected", "deprecated"],
        }
    }

    pub fn allows_state(self, state: &str) -> bool {
        self.allowed_states().contains(&state)
    }
}

/// Find the shortest path from `from` to `to` via BFS over the unified
/// transition graph, restricted to states allowed for `lifecycle`.
///
/// Inputs may be 1.x state names; they are normalized internally so legacy
/// callers (CLI shorthands, replay of pre-2.0 events) keep working.
pub fn find_path(lifecycle: Lifecycle, from: &str, to: &str) -> Option<Vec<&'static str>> {
    let from = normalize_state_name(from);
    let to = normalize_state_name(to);
    if from == to {
        return Some(vec![]);
    }
    if !lifecycle.allows_state(to) {
        return None;
    }
    let mut queue: VecDeque<(&str, Vec<&'static str>)> = VecDeque::new();
    let mut visited: Vec<&str> = vec![from];

    for &(src, dst) in UNIFIED_TRANSITIONS {
        if src == from && lifecycle.allows_state(dst) {
            if dst == to {
                return Some(vec![dst]);
            }
            visited.push(dst);
            queue.push_back((dst, vec![dst]));
        }
    }

    while let Some((current, path)) = queue.pop_front() {
        for &(src, dst) in UNIFIED_TRANSITIONS {
            if src == current && lifecycle.allows_state(dst) && !visited.contains(&dst) {
                let mut new_path = path.clone();
                new_path.push(dst);
                if dst == to {
                    return Some(new_path);
                }
                visited.push(dst);
                queue.push_back((dst, new_path));
            }
        }
    }
    None
}

/// Whether `from -> to` is a valid edge for the given lifecycle.
///
/// Inputs may be 1.x state names; they are normalized internally. Both
/// endpoints must be in the lifecycle's allowed set (§3.1.1) and the edge
/// must exist in the unified §3.1 graph.
pub fn is_valid_transition(lifecycle: Lifecycle, from: &str, to: &str) -> bool {
    let from = normalize_state_name(from);
    let to = normalize_state_name(to);
    lifecycle.allows_state(from)
        && lifecycle.allows_state(to)
        && UNIFIED_TRANSITIONS
            .iter()
            .any(|&(s, d)| s == from && d == to)
}

/// Destination states reachable in one step from `from` for the given lifecycle.
///
/// Returns 2.0 state names. The input may be a 1.x state name; it is
/// normalized internally.
pub fn valid_targets(lifecycle: Lifecycle, from: &str) -> Vec<&'static str> {
    let from = normalize_state_name(from);
    UNIFIED_TRANSITIONS
        .iter()
        .filter_map(|&(s, d)| (s == from && lifecycle.allows_state(d)).then_some(d))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- SPEC-2.0 §3.1 unified machine ----

    #[test]
    fn unified_proposal_typical_path() {
        // SPEC-2.0 §3.1.1 typical path for proposal: draft → open → review → done
        assert!(is_valid_transition(Lifecycle::Proposal, "draft", "open"));
        assert!(is_valid_transition(Lifecycle::Proposal, "open", "review"));
        assert!(is_valid_transition(Lifecycle::Proposal, "review", "done"));
    }

    #[test]
    fn unified_execution_excludes_withdrawn() {
        // §3.1.1: execution allowed states do NOT include withdrawn.
        assert!(!Lifecycle::Execution.allows_state("withdrawn"));
        assert!(!is_valid_transition(
            Lifecycle::Execution,
            "open",
            "withdrawn"
        ));
    }

    #[test]
    fn unified_record_excludes_review_and_working() {
        assert!(!Lifecycle::Record.allows_state("working"));
        assert!(!Lifecycle::Record.allows_state("review"));
        assert!(!is_valid_transition(Lifecycle::Record, "open", "review"));
        assert!(is_valid_transition(Lifecycle::Record, "open", "done"));
    }

    #[test]
    fn unified_proposal_excludes_working() {
        // §3.1.1: proposals don't have a "doing the work" state.
        assert!(!Lifecycle::Proposal.allows_state("working"));
    }

    #[test]
    fn find_path_proposal_draft_to_done() {
        // BFS picks shortest: draft → open → done (skips review).
        assert_eq!(
            find_path(Lifecycle::Proposal, "draft", "done"),
            Some(vec!["open", "done"])
        );
    }

    #[test]
    fn find_path_proposal_draft_to_review() {
        assert_eq!(
            find_path(Lifecycle::Proposal, "draft", "review"),
            Some(vec!["open", "review"])
        );
    }

    #[test]
    fn find_path_execution_open_to_done_picks_shortest() {
        assert_eq!(
            find_path(Lifecycle::Execution, "open", "done"),
            Some(vec!["done"])
        );
    }

    #[test]
    fn find_path_same_state_returns_empty() {
        assert_eq!(
            find_path(Lifecycle::Proposal, "draft", "draft"),
            Some(vec![])
        );
    }

    #[test]
    fn find_path_unreachable_returns_none() {
        assert_eq!(find_path(Lifecycle::Proposal, "done", "draft"), None);
    }

    #[test]
    fn find_path_bogus_target_returns_none() {
        assert_eq!(find_path(Lifecycle::Proposal, "draft", "bogus"), None);
    }

    #[test]
    fn lifecycle_initial_states() {
        assert_eq!(Lifecycle::Proposal.initial_state(), "draft");
        assert_eq!(Lifecycle::Execution.initial_state(), "open");
        assert_eq!(Lifecycle::Record.initial_state(), "open");
    }

    #[test]
    fn valid_targets_proposal_from_draft() {
        // Only draft→open and draft→withdrawn exist in the unified graph.
        assert_eq!(
            valid_targets(Lifecycle::Proposal, "draft"),
            vec!["open", "withdrawn"]
        );
    }

    #[test]
    fn valid_targets_execution_from_open() {
        // open → working/review/done/rejected. withdrawn excluded by execution.
        assert_eq!(
            valid_targets(Lifecycle::Execution, "open"),
            vec!["working", "review", "done", "rejected"]
        );
    }

    // ---- 1.x state-name normalization at boundaries ----

    #[test]
    fn legacy_state_names_are_normalized_in_queries() {
        // Caller passes 1.x names; the query layer normalizes before checking.
        assert!(is_valid_transition(
            Lifecycle::Proposal,
            "under-review",
            "accepted"
        ));
        assert!(is_valid_transition(Lifecycle::Execution, "open", "closed"));
        assert!(is_valid_transition(
            Lifecycle::Execution,
            "designing",
            "reviewing"
        ));
    }

    #[test]
    fn find_path_accepts_legacy_state_names() {
        // proposed (= open) → accepted (= done) for proposal
        assert_eq!(
            find_path(Lifecycle::Proposal, "proposed", "accepted"),
            Some(vec!["done"])
        );
    }

    // ---- SPEC-2.0 §3.1.2 1.x→2.0 round-trip ----

    /// Every state reachable in any 1.x kind's transition table — the union
    /// of the four legacy state machines, exercised by the round-trip test.
    fn all_1x_states() -> &'static [(ThreadKind, &'static str)] {
        &[
            (ThreadKind::Issue, "open"),
            (ThreadKind::Issue, "pending"),
            (ThreadKind::Issue, "closed"),
            (ThreadKind::Issue, "rejected"),
            (ThreadKind::Issue, "withdrawn"),
            (ThreadKind::Rfc, "draft"),
            (ThreadKind::Rfc, "proposed"),
            (ThreadKind::Rfc, "under-review"),
            (ThreadKind::Rfc, "accepted"),
            (ThreadKind::Rfc, "rejected"),
            (ThreadKind::Rfc, "withdrawn"),
            (ThreadKind::Rfc, "deprecated"),
            (ThreadKind::Dec, "proposed"),
            (ThreadKind::Dec, "accepted"),
            (ThreadKind::Dec, "rejected"),
            (ThreadKind::Dec, "deprecated"),
            (ThreadKind::Dec, "withdrawn"),
            (ThreadKind::Task, "open"),
            (ThreadKind::Task, "designing"),
            (ThreadKind::Task, "implementing"),
            (ThreadKind::Task, "reviewing"),
            (ThreadKind::Task, "closed"),
            (ThreadKind::Task, "rejected"),
            (ThreadKind::Task, "withdrawn"),
        ]
    }

    #[test]
    fn round_trip_every_1x_kind_state_lands_in_lifecycle_allowed_set() {
        // Acceptance criterion (JOB-41f5guw8): every (1.x kind, 1.x state)
        // pair migrates to a valid 2.0 (lifecycle, state) pair.
        for &(kind, state) in all_1x_states() {
            let lifecycle = kind.lifecycle();
            let migrated = migrate_legacy_state(kind, state);
            assert!(
                lifecycle.allows_state(migrated),
                "kind={kind} state={state} migrated={migrated} \
                 lifecycle={lifecycle} allowed={:?}",
                lifecycle.allowed_states(),
            );
        }
    }

    #[test]
    fn migrate_drops_withdrawn_for_execution() {
        assert_eq!(
            migrate_legacy_state(ThreadKind::Issue, "withdrawn"),
            "rejected"
        );
        assert_eq!(
            migrate_legacy_state(ThreadKind::Task, "withdrawn"),
            "rejected"
        );
        assert_eq!(
            migrate_legacy_state(ThreadKind::Rfc, "withdrawn"),
            "withdrawn"
        );
    }

    #[test]
    fn normalize_known_legacy_state_names() {
        assert_eq!(normalize_state_name("accepted"), "done");
        assert_eq!(normalize_state_name("closed"), "done");
        assert_eq!(normalize_state_name("under-review"), "review");
        assert_eq!(normalize_state_name("reviewing"), "review");
        assert_eq!(normalize_state_name("proposed"), "open");
        assert_eq!(normalize_state_name("pending"), "working");
        assert_eq!(normalize_state_name("designing"), "working");
        assert_eq!(normalize_state_name("implementing"), "working");
        assert_eq!(normalize_state_name("draft"), "draft");
        assert_eq!(normalize_state_name("open"), "open");
        assert_eq!(normalize_state_name("rejected"), "rejected");
        assert_eq!(normalize_state_name("withdrawn"), "withdrawn");
        assert_eq!(normalize_state_name("deprecated"), "deprecated");
    }
}
