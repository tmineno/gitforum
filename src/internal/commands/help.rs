//! `git forum --help-llm` orchestration.
//!
//! Phase 2 slot 10e (RFC `7ymtc4b2`): the `--help-llm` dispatcher.
//! Library code (`internal::help::*` — `node_type_taxonomy`,
//! `state_transition_map`, `evidence_kinds_reference`) stays
//! peer-level since several callers consume it; this module owns the
//! CLI-only context-routing decision and the inclusion of the full
//! `doc/MANUAL.md` payload. The routing map only references 3.0
//! subcommands — the 1.x rhetorical shorthands (`claim`, `question`,
//! `summary`, `risk`, `review`, `alternative`, `assumption`) are
//! removed in 3.0 (SPEC-3.0 §2.2 / ADR-006), so they no longer have
//! a focused-help target.

use crate::internal::help;

/// Print the LLM manual (or a context-specific section) and return.
///
/// `context` is the token immediately before `--help-llm` on the CLI
/// (e.g. `git forum node --help-llm` → `Some("node")`). When it
/// matches a known cluster, print the corresponding focused section;
/// otherwise print the full manual.
pub fn run(context: Option<&str>) {
    match context {
        // Discussion cluster: `node` plus the three canonical 3.0
        // node-add shorthands.
        Some("node" | "comment" | "objection" | "action") => {
            print!("{}", help::node_type_taxonomy());
        }
        // State-transition cluster: the generic `state` arm plus the
        // category-aware shorthands.
        Some(
            "state" | "close" | "reject" | "accept" | "propose" | "deprecate" | "pend" | "withdraw",
        ) => {
            print!("{}", help::state_transition_map());
        }
        Some("evidence") => {
            print!("{}", help::evidence_kinds_reference());
        }
        _ => {
            print!("{}", include_str!("../../../doc/MANUAL.md"));
        }
    }
}
