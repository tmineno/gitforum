//! `git forum --help-llm` orchestration.
//!
//! Phase 2 slot 10e (RFC `7ymtc4b2`): NEW module owning the
//! `--help-llm` dispatcher. Library code (`internal::help::*` —
//! `node_type_taxonomy`, `state_transition_map`,
//! `evidence_kinds_reference`) stays peer-level; this module is the
//! CLI-only context-routing handler. Vocabulary in the library
//! itself is updated to 3.0 categories (kind / facet / lifecycle
//! removal) as a separate concern of `internal::help`.

use crate::internal::help;

/// Print the LLM manual (or a context-specific section) and return.
///
/// `context` is the token immediately before `--help-llm` on the CLI
/// (e.g. `git forum claim --help-llm` → `Some("claim")`). When it
/// matches a known cluster, print the corresponding focused section;
/// otherwise print the full manual.
pub fn run(context: Option<&str>) {
    match context {
        Some(
            "claim" | "question" | "objection" | "summary" | "action" | "risk" | "review"
            | "alternative" | "assumption" | "node",
        ) => {
            print!("{}", help::node_type_taxonomy());
        }
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
