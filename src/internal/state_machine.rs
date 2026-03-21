use super::event::ThreadKind;

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
            ("pending", "closed"),
            ("pending", "open"),
            ("closed", "open"),
            ("rejected", "open"),
        ],
        ThreadKind::Rfc => &[
            ("draft", "proposed"),
            ("draft", "rejected"),
            ("proposed", "under-review"),
            ("proposed", "draft"),
            ("under-review", "accepted"),
            ("under-review", "rejected"),
            ("under-review", "draft"),
            ("accepted", "deprecated"),
            ("rejected", "deprecated"),
        ],
        ThreadKind::Dec => &[
            ("proposed", "accepted"),
            ("proposed", "rejected"),
            ("proposed", "deprecated"),
            ("accepted", "deprecated"),
            ("rejected", "deprecated"),
        ],
        ThreadKind::Task => &[
            ("open", "designing"),
            ("open", "rejected"),
            ("open", "closed"),
            ("designing", "implementing"),
            ("designing", "rejected"),
            ("designing", "open"),
            ("implementing", "reviewing"),
            ("implementing", "rejected"),
            ("implementing", "designing"),
            ("reviewing", "closed"),
            ("reviewing", "rejected"),
            ("reviewing", "implementing"),
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
            vec!["proposed", "rejected"]
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
        assert_eq!(targets, vec!["accepted", "rejected", "deprecated"]);
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
        assert_eq!(targets, vec!["designing", "rejected", "closed"]);
    }
}
