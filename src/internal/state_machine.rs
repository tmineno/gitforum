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

fn valid_transitions(kind: ThreadKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        ThreadKind::Issue => &[
            ("open", "closed"),
            ("open", "rejected"),
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
}
