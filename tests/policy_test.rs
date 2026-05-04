//! Policy parser/lint integration tests.
//!
//! The bulk of policy-shape coverage now lives inside
//! `src/internal/policy.rs` (`policy_load_tests`, `legacy_detection_tests`,
//! `evaluate_tests`, `category_registry_tests`) — moved there with the
//! SPEC-3.0 §3.2/§3.3 rewrite (task `9635buy0`, pre-flight P5). This
//! file retains a single integration-level smoke test that the parser
//! reads `.forum/policy.toml` from a real init.

mod support;

use git_forum::internal::policy::Policy;
use support::forum::setup;

#[test]
fn policy_loads_from_initialized_repo() {
    let (_repo, _git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap();
    assert!(policy.category("rfc").is_some());
    assert!(policy.category("task").is_some());
}
