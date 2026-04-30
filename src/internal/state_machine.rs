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
pub fn find_path_lifecycle(
    lifecycle: Lifecycle,
    from: &str,
    to: &str,
) -> Option<Vec<&'static str>> {
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
pub fn is_valid_transition_lifecycle(lifecycle: Lifecycle, from: &str, to: &str) -> bool {
    lifecycle.allows_state(to)
        && UNIFIED_TRANSITIONS
            .iter()
            .any(|&(s, d)| s == from && d == to)
}

/// Destination states reachable in one step from `from` for the given lifecycle.
pub fn valid_targets_lifecycle(lifecycle: Lifecycle, from: &str) -> Vec<&'static str> {
    UNIFIED_TRANSITIONS
        .iter()
        .filter_map(|&(s, d)| (s == from && lifecycle.allows_state(d)).then_some(d))
        .collect()
}

/// Find the shortest path from `from` to `to` via BFS over valid transitions.
///
/// Preconditions: from and to are non-empty strings.
/// Postconditions: returns Some(vec of intermediate + final states) if reachable, None otherwise.
///   The returned path excludes `from` and includes `to`.
/// Failure modes: none (returns Option).
/// Side effects: none.
pub fn find_path(kind: ThreadKind, from: &str, to: &str) -> Option<Vec<&'static str>> {
    if from == to {
        return Some(vec![]);
    }
    let transitions = valid_transitions(kind);
    let mut queue: VecDeque<(&str, Vec<&'static str>)> = VecDeque::new();
    let mut visited: Vec<&str> = vec![from];

    // Seed with direct neighbours of `from`
    for &(src, dst) in transitions {
        if src == from {
            if dst == to {
                return Some(vec![dst]);
            }
            visited.push(dst);
            queue.push_back((dst, vec![dst]));
        }
    }

    while let Some((current, path)) = queue.pop_front() {
        for &(src, dst) in transitions {
            if src == current && !visited.contains(&dst) {
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

/// Check whether a state transition is valid for the given thread kind.
///
/// Preconditions: from and to are non-empty strings.
/// Postconditions: returns true iff the transition is listed.
/// Failure modes: none (returns bool).
/// Side effects: none.
pub fn is_valid_transition(kind: ThreadKind, from: &str, to: &str) -> bool {
    valid_transitions(kind).contains(&(from, to))
}

/// Return valid destination states from the current state.
pub fn valid_targets(kind: ThreadKind, from: &str) -> Vec<&'static str> {
    valid_transitions(kind)
        .iter()
        .filter_map(|(src, dst)| (*src == from).then_some(*dst))
        .collect()
}

pub fn valid_transitions(kind: ThreadKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        ThreadKind::Issue => &[
            ("open", "pending"),
            ("open", "closed"),
            ("open", "rejected"),
            ("open", "withdrawn"),
            ("pending", "closed"),
            ("pending", "open"),
            ("pending", "withdrawn"),
            ("closed", "open"),
            ("rejected", "open"),
        ],
        ThreadKind::Rfc => &[
            ("draft", "proposed"),
            ("draft", "rejected"),
            ("draft", "withdrawn"),
            ("proposed", "under-review"),
            ("proposed", "draft"),
            ("proposed", "withdrawn"),
            ("under-review", "accepted"),
            ("under-review", "rejected"),
            ("under-review", "draft"),
            ("under-review", "withdrawn"),
            ("accepted", "deprecated"),
            ("rejected", "deprecated"),
        ],
        ThreadKind::Dec => &[
            ("proposed", "accepted"),
            ("proposed", "rejected"),
            ("proposed", "deprecated"),
            ("proposed", "withdrawn"),
            ("accepted", "deprecated"),
            ("rejected", "deprecated"),
        ],
        ThreadKind::Task => &[
            ("open", "designing"),
            ("open", "rejected"),
            ("open", "closed"),
            ("open", "withdrawn"),
            ("designing", "implementing"),
            ("designing", "rejected"),
            ("designing", "open"),
            ("designing", "withdrawn"),
            ("implementing", "reviewing"),
            ("implementing", "rejected"),
            ("implementing", "designing"),
            ("implementing", "withdrawn"),
            ("reviewing", "closed"),
            ("reviewing", "rejected"),
            ("reviewing", "implementing"),
            ("reviewing", "withdrawn"),
            ("closed", "open"),
            ("rejected", "open"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_draft_to_proposed_is_valid() {
        assert!(is_valid_transition(ThreadKind::Rfc, "draft", "proposed"));
    }

    #[test]
    fn rfc_draft_to_accepted_is_invalid() {
        assert!(!is_valid_transition(ThreadKind::Rfc, "draft", "accepted"));
    }

    #[test]
    fn issue_open_to_closed_is_valid() {
        assert!(is_valid_transition(ThreadKind::Issue, "open", "closed"));
    }

    #[test]
    fn issue_closed_can_reopen() {
        assert!(is_valid_transition(ThreadKind::Issue, "closed", "open"));
    }

    #[test]
    fn rfc_under_review_to_accepted_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Rfc,
            "under-review",
            "accepted"
        ));
    }

    #[test]
    fn rfc_proposed_to_under_review_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Rfc,
            "proposed",
            "under-review"
        ));
    }

    #[test]
    fn valid_targets_lists_expected_rfc_transitions() {
        assert_eq!(
            valid_targets(ThreadKind::Rfc, "draft"),
            vec!["proposed", "rejected", "withdrawn"]
        );
    }

    #[test]
    fn bogus_transition_is_invalid() {
        assert!(!is_valid_transition(ThreadKind::Rfc, "draft", "bogus"));
    }

    #[test]
    fn issue_open_to_rejected_is_valid() {
        assert!(is_valid_transition(ThreadKind::Issue, "open", "rejected"));
    }

    #[test]
    fn issue_rejected_to_open_is_valid() {
        assert!(is_valid_transition(ThreadKind::Issue, "rejected", "open"));
    }

    #[test]
    fn rfc_accepted_to_deprecated_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Rfc,
            "accepted",
            "deprecated"
        ));
    }

    #[test]
    fn rfc_rejected_to_deprecated_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Rfc,
            "rejected",
            "deprecated"
        ));
    }

    #[test]
    fn issue_open_to_pending_is_valid() {
        assert!(is_valid_transition(ThreadKind::Issue, "open", "pending"));
    }

    #[test]
    fn issue_pending_to_closed_is_valid() {
        assert!(is_valid_transition(ThreadKind::Issue, "pending", "closed"));
    }

    #[test]
    fn issue_pending_to_open_is_valid() {
        assert!(is_valid_transition(ThreadKind::Issue, "pending", "open"));
    }

    #[test]
    fn issue_pending_to_rejected_is_invalid() {
        assert!(!is_valid_transition(
            ThreadKind::Issue,
            "pending",
            "rejected"
        ));
    }

    // --- DEC transitions ---

    #[test]
    fn dec_proposed_to_accepted_is_valid() {
        assert!(is_valid_transition(ThreadKind::Dec, "proposed", "accepted"));
    }

    #[test]
    fn dec_proposed_to_rejected_is_valid() {
        assert!(is_valid_transition(ThreadKind::Dec, "proposed", "rejected"));
    }

    #[test]
    fn dec_proposed_to_deprecated_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Dec,
            "proposed",
            "deprecated"
        ));
    }

    #[test]
    fn dec_accepted_to_deprecated_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Dec,
            "accepted",
            "deprecated"
        ));
    }

    #[test]
    fn dec_rejected_to_deprecated_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Dec,
            "rejected",
            "deprecated"
        ));
    }

    #[test]
    fn dec_accepted_to_proposed_is_invalid() {
        assert!(!is_valid_transition(
            ThreadKind::Dec,
            "accepted",
            "proposed"
        ));
    }

    #[test]
    fn dec_valid_targets_from_proposed() {
        let targets = valid_targets(ThreadKind::Dec, "proposed");
        assert_eq!(
            targets,
            vec!["accepted", "rejected", "deprecated", "withdrawn"]
        );
    }

    // --- TASK transitions ---

    #[test]
    fn task_open_to_designing_is_valid() {
        assert!(is_valid_transition(ThreadKind::Task, "open", "designing"));
    }

    #[test]
    fn task_fast_track_open_to_closed_is_valid() {
        assert!(is_valid_transition(ThreadKind::Task, "open", "closed"));
    }

    #[test]
    fn task_full_lifecycle() {
        assert!(is_valid_transition(ThreadKind::Task, "open", "designing"));
        assert!(is_valid_transition(
            ThreadKind::Task,
            "designing",
            "implementing"
        ));
        assert!(is_valid_transition(
            ThreadKind::Task,
            "implementing",
            "reviewing"
        ));
        assert!(is_valid_transition(ThreadKind::Task, "reviewing", "closed"));
    }

    #[test]
    fn task_back_transitions() {
        assert!(is_valid_transition(
            ThreadKind::Task,
            "implementing",
            "designing"
        ));
        assert!(is_valid_transition(
            ThreadKind::Task,
            "reviewing",
            "implementing"
        ));
    }

    #[test]
    fn task_reopen_from_closed() {
        assert!(is_valid_transition(ThreadKind::Task, "closed", "open"));
    }

    #[test]
    fn task_reopen_from_rejected() {
        assert!(is_valid_transition(ThreadKind::Task, "rejected", "open"));
    }

    #[test]
    fn task_open_to_reviewing_is_invalid() {
        assert!(!is_valid_transition(ThreadKind::Task, "open", "reviewing"));
    }

    #[test]
    fn task_reviewing_to_designing_is_invalid() {
        assert!(!is_valid_transition(
            ThreadKind::Task,
            "reviewing",
            "designing"
        ));
    }

    #[test]
    fn task_valid_targets_from_open() {
        let targets = valid_targets(ThreadKind::Task, "open");
        assert_eq!(
            targets,
            vec!["designing", "rejected", "closed", "withdrawn"]
        );
    }

    // --- find_path tests ---

    #[test]
    fn find_path_same_state_returns_empty() {
        assert_eq!(find_path(ThreadKind::Rfc, "draft", "draft"), Some(vec![]));
    }

    #[test]
    fn find_path_direct_transition() {
        assert_eq!(
            find_path(ThreadKind::Rfc, "draft", "proposed"),
            Some(vec!["proposed"])
        );
    }

    #[test]
    fn find_path_rfc_draft_to_accepted() {
        assert_eq!(
            find_path(ThreadKind::Rfc, "draft", "accepted"),
            Some(vec!["proposed", "under-review", "accepted"])
        );
    }

    #[test]
    fn find_path_unreachable_returns_none() {
        // accepted -> draft has no forward path (only back via under-review->draft)
        // Actually under-review->draft exists, but accepted has no path to under-review
        assert_eq!(find_path(ThreadKind::Rfc, "accepted", "draft"), None);
    }

    #[test]
    fn find_path_task_open_to_closed_picks_shortest() {
        // Direct edge open->closed exists; should not go through designing->implementing->reviewing
        assert_eq!(
            find_path(ThreadKind::Task, "open", "closed"),
            Some(vec!["closed"])
        );
    }

    #[test]
    fn find_path_task_open_to_reviewing() {
        assert_eq!(
            find_path(ThreadKind::Task, "open", "reviewing"),
            Some(vec!["designing", "implementing", "reviewing"])
        );
    }

    #[test]
    fn find_path_issue_open_to_closed() {
        assert_eq!(
            find_path(ThreadKind::Issue, "open", "closed"),
            Some(vec!["closed"])
        );
    }

    #[test]
    fn find_path_dec_proposed_to_deprecated() {
        // Direct edge exists
        assert_eq!(
            find_path(ThreadKind::Dec, "proposed", "deprecated"),
            Some(vec!["deprecated"])
        );
    }

    #[test]
    fn find_path_bogus_target_returns_none() {
        assert_eq!(find_path(ThreadKind::Rfc, "draft", "bogus"), None);
    }

    // ---- SPEC-2.0 §3.1 unified machine ----

    #[test]
    fn unified_proposal_typical_path() {
        // SPEC-2.0 §3.1.1 typical path for proposal: draft → open → review → done
        assert!(is_valid_transition_lifecycle(
            Lifecycle::Proposal,
            "draft",
            "open"
        ));
        assert!(is_valid_transition_lifecycle(
            Lifecycle::Proposal,
            "open",
            "review"
        ));
        assert!(is_valid_transition_lifecycle(
            Lifecycle::Proposal,
            "review",
            "done"
        ));
    }

    #[test]
    fn unified_execution_excludes_withdrawn() {
        // §3.1.1: execution allowed states do NOT include withdrawn.
        assert!(!Lifecycle::Execution.allows_state("withdrawn"));
        assert!(!is_valid_transition_lifecycle(
            Lifecycle::Execution,
            "open",
            "withdrawn"
        ));
    }

    #[test]
    fn unified_record_excludes_review_and_working() {
        assert!(!Lifecycle::Record.allows_state("working"));
        assert!(!Lifecycle::Record.allows_state("review"));
        assert!(!is_valid_transition_lifecycle(
            Lifecycle::Record,
            "open",
            "review"
        ));
        // open -> done is the typical record path
        assert!(is_valid_transition_lifecycle(
            Lifecycle::Record,
            "open",
            "done"
        ));
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
            find_path_lifecycle(Lifecycle::Proposal, "draft", "done"),
            Some(vec!["open", "done"])
        );
    }

    #[test]
    fn find_path_proposal_draft_to_review() {
        assert_eq!(
            find_path_lifecycle(Lifecycle::Proposal, "draft", "review"),
            Some(vec!["open", "review"])
        );
    }

    #[test]
    fn find_path_execution_open_to_done_picks_shortest() {
        // open->done is a direct edge.
        assert_eq!(
            find_path_lifecycle(Lifecycle::Execution, "open", "done"),
            Some(vec!["done"])
        );
    }

    #[test]
    fn lifecycle_initial_states() {
        assert_eq!(Lifecycle::Proposal.initial_state(), "draft");
        assert_eq!(Lifecycle::Execution.initial_state(), "open");
        assert_eq!(Lifecycle::Record.initial_state(), "open");
    }

    // ---- SPEC-2.0 §3.1.2 1.x→2.0 round-trip ----

    fn legacy_states_for(kind: ThreadKind) -> Vec<&'static str> {
        valid_transitions(kind)
            .iter()
            .flat_map(|(f, t)| [*f, *t])
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[test]
    fn round_trip_every_1x_kind_state_lands_in_lifecycle_allowed_set() {
        // Acceptance criterion (JOB-41f5guw8): every (1.x kind, 1.x state)
        // round-trips to a valid 2.0 (lifecycle, state) pair.
        for kind in [
            ThreadKind::Issue,
            ThreadKind::Rfc,
            ThreadKind::Dec,
            ThreadKind::Task,
        ] {
            let lifecycle = kind.lifecycle();
            for state in legacy_states_for(kind) {
                let migrated = migrate_legacy_state(kind, state);
                assert!(
                    lifecycle.allows_state(migrated),
                    "kind={kind} state={state} migrated={migrated} \
                     lifecycle={lifecycle} allowed={:?}",
                    lifecycle.allowed_states(),
                );
            }
        }
    }

    #[test]
    fn migrate_drops_withdrawn_for_execution() {
        // §3.1.1: execution does not allow withdrawn, so 1.x Issue/Task
        // `withdrawn` remap to `rejected`.
        assert_eq!(
            migrate_legacy_state(ThreadKind::Issue, "withdrawn"),
            "rejected"
        );
        assert_eq!(
            migrate_legacy_state(ThreadKind::Task, "withdrawn"),
            "rejected"
        );
        // Proposal allows withdrawn — passes through.
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
        // 2.0 names and shared 1.x names pass through.
        assert_eq!(normalize_state_name("draft"), "draft");
        assert_eq!(normalize_state_name("open"), "open");
        assert_eq!(normalize_state_name("rejected"), "rejected");
        assert_eq!(normalize_state_name("withdrawn"), "withdrawn");
        assert_eq!(normalize_state_name("deprecated"), "deprecated");
    }
}
