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

fn valid_transitions(kind: ThreadKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        ThreadKind::Issue => &[("open", "closed"), ("closed", "open")],
        ThreadKind::Rfc => &[
            ("draft", "under-review"),
            ("under-review", "accepted"),
            ("under-review", "rejected"),
            ("under-review", "draft"),
            ("draft", "rejected"),
        ],
        ThreadKind::Decision => &[
            ("proposed", "accepted"),
            ("proposed", "rejected"),
            ("accepted", "superseded"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_draft_to_under_review_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Rfc,
            "draft",
            "under-review"
        ));
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
    fn decision_proposed_to_accepted_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Decision,
            "proposed",
            "accepted"
        ));
    }

    #[test]
    fn decision_accepted_to_superseded_is_valid() {
        assert!(is_valid_transition(
            ThreadKind::Decision,
            "accepted",
            "superseded"
        ));
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
    fn bogus_transition_is_invalid() {
        assert!(!is_valid_transition(ThreadKind::Rfc, "draft", "bogus"));
    }
}
