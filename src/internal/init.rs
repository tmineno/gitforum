use std::fs;

use super::config::RepoPaths;
use super::error::ForumResult;

const DEFAULT_POLICY: &str = r#"# git-forum default policy
#
# This file has two sections:
#   1. Transition guards — conditions that must be met before a state change.
#   2. Operation checks  — rules about what is allowed in each state.
#
# Transitions without a kind prefix are GLOBAL — "open->closed" applies to
# every kind that has both states (issue AND task).
#
# Kind-scoped guards (optional prefix):
#   on = "dec:proposed->accepted"   — only applies to DEC threads
#   on = "proposed->accepted"       — applies to all kinds with this transition
#
# When both a scoped and unscoped guard match, both apply (union semantics).
# If you need different rules per kind, use kind-scoped keys.
#
# State names per thread kind:
#   issue: open, pending, closed, rejected
#   rfc:   draft, proposed, under-review, accepted, rejected, deprecated
#   dec:   proposed, accepted, rejected, deprecated
#   task:  open, designing, implementing, reviewing, closed, rejected
#
# Shared states: "rejected" appears in all kinds; "open"/"closed" in issue
# and task; "proposed"/"accepted"/"deprecated" in rfc and dec.

# ═══════════════════════════════════════════════════════════════════════
# 1. TRANSITION GUARDS
# ═══════════════════════════════════════════════════════════════════════
# Each [[guards]] block defines conditions ("requires") that must ALL pass
# before the transition in "on" is allowed.
#
# A guard violation is always an error — the transition is blocked.
# This is different from operation checks (below), which can be warnings.
#
# Available guard rules:
#   no_open_objections  — all objection nodes must be resolved/retracted
#   no_open_actions     — all action nodes must be resolved/retracted
#   at_least_one_summary — thread must have a non-retracted summary node
#   one_human_approval  — at least one human/… actor approval required
#   has_commit_evidence  — thread must have commit-type evidence attached

[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]

[[guards]]
on = "open->closed"
requires = ["no_open_actions"]

[[guards]]
on = "proposed->accepted"
requires = ["no_open_objections"]

[[guards]]
on = "reviewing->closed"
requires = ["no_open_actions"]

# Uncomment to require commit evidence before closing issues:
# [[guards]]
# on = "open->closed"
# requires = ["has_commit_evidence"]

# ═══════════════════════════════════════════════════════════════════════
# 2. OPERATION CHECKS
# ═══════════════════════════════════════════════════════════════════════
# Operation checks validate creation, node posting, revision, and evidence
# operations. Unlike guards, checks have two severity levels:
#
#   strict = false (default) — violations are WARNINGS; use --force to bypass
#   strict = true            — violations are ERRORS; operation is blocked
#
# State names in operation checks (node_rules, revise_rules, evidence_rules)
# are matched globally, just like guard transitions. A state name like
# "closed" applies to any thread kind that uses that state.

[checks]
strict = false

# Creation rules: what is required when creating a new thread.
# Keyed by thread kind (rfc, issue, dec, task).
[creation_rules.rfc]
required_body = true
body_sections = ["Goal", "Non-goals", "Context", "Proposal"]

[creation_rules.issue]
required_body = false
body_sections = []

[creation_rules.dec]
required_body = true
body_sections = ["Context", "Decision", "Rationale", "Impact"]

[creation_rules.task]
required_body = false
body_sections = ["Background", "Acceptance criteria", "Exceptions"]

# Node rules: which node types are allowed in each state.
# State names are global — "rejected" affects issue, rfc, dec, and task.
# No restrictions by default — all node types allowed in all states.
# Uncomment to restrict node types in terminal states:
# [node_rules]
# "accepted" = []
# "closed" = []
# "rejected" = []
# "deprecated" = []

# Revise rules: in which states body/node revision is allowed.
# State names listed here are global across all thread kinds.
[revise_rules]
allow_body_revise = ["draft", "proposed", "open", "pending", "designing", "implementing"]
allow_node_revise = ["draft", "proposed", "under-review", "open", "pending", "designing", "implementing", "reviewing"]

# Evidence rules: in which states evidence can be attached.
# State names listed here are global across all thread kinds.
[evidence_rules]
allow_evidence = ["draft", "proposed", "under-review", "open", "pending", "designing", "implementing", "reviewing", "closed", "accepted", "rejected", "deprecated"]
"#;

const DEFAULT_ACTORS: &str = r#"# git-forum actors
# Register human and AI actors here.
#
# [[actors]]
# id = "human/alice"
# kind = "human"
# display_name = "Alice"
"#;

const TEMPLATE_ISSUE: &str = "# {title}\n";
const TEMPLATE_RFC: &str = "\
# {title}

## Goal

## Non-goals

## Context

## Proposal
";
const TEMPLATE_DEC: &str = "\
# {title}

## Context

## Decision

## Rationale

## Impact
";
const TEMPLATE_TASK: &str = "\
# {title}

## Background

## Acceptance criteria

## Exceptions
";

/// Initialize `.forum/` and `.git/forum/` structure.
///
/// Idempotent: skips files that already exist.
pub fn init_forum(paths: &RepoPaths) -> ForumResult<()> {
    // .forum/ shared config
    let templates_dir = paths.dot_forum.join("templates");
    fs::create_dir_all(&templates_dir)?;

    write_if_missing(&paths.dot_forum.join("policy.toml"), DEFAULT_POLICY)?;
    write_if_missing(&paths.dot_forum.join("actors.toml"), DEFAULT_ACTORS)?;
    write_if_missing(&templates_dir.join("issue.md"), TEMPLATE_ISSUE)?;
    write_if_missing(&templates_dir.join("rfc.md"), TEMPLATE_RFC)?;
    write_if_missing(&templates_dir.join("dec.md"), TEMPLATE_DEC)?;
    write_if_missing(&templates_dir.join("task.md"), TEMPLATE_TASK)?;

    // .git/forum/ local-only data
    fs::create_dir_all(paths.git_forum.join("logs"))?;

    // Set up a local git alias so `git forum --help` works correctly
    if let Some(repo_root) = paths.dot_forum.parent() {
        let _ = std::process::Command::new("git")
            .args(["config", "--local", "alias.forum", "!git-forum"])
            .current_dir(repo_root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status();
    }

    Ok(())
}

fn write_if_missing(path: &std::path::Path, content: &str) -> ForumResult<()> {
    if !path.exists() {
        fs::write(path, content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::policy::Policy;

    #[test]
    fn default_policy_deserializes() {
        let policy: Policy =
            toml::from_str(DEFAULT_POLICY).expect("DEFAULT_POLICY must be valid TOML");

        // Guards
        assert_eq!(policy.guards.len(), 4);
        assert_eq!(policy.guards[0].on, "under-review->accepted");
        assert_eq!(policy.guards[1].on, "open->closed");
        assert_eq!(policy.guards[2].on, "proposed->accepted");
        assert_eq!(policy.guards[3].on, "reviewing->closed");

        // Checks config
        assert!(!policy.checks.strict);

        // Creation rules — RFC
        let rfc_rules = policy
            .creation_rules
            .get("rfc")
            .expect("creation_rules.rfc must exist");
        assert!(rfc_rules.required_body);
        assert_eq!(
            rfc_rules.body_sections,
            vec!["Goal", "Non-goals", "Context", "Proposal"]
        );

        // Creation rules — issue
        let issue_rules = policy
            .creation_rules
            .get("issue")
            .expect("creation_rules.issue must exist");
        assert!(!issue_rules.required_body);
        assert!(issue_rules.body_sections.is_empty());

        // Creation rules — dec
        let dec_rules = policy
            .creation_rules
            .get("dec")
            .expect("creation_rules.dec must exist");
        assert!(dec_rules.required_body);
        assert_eq!(
            dec_rules.body_sections,
            vec!["Context", "Decision", "Rationale", "Impact"]
        );

        // Creation rules — task
        let task_rules = policy
            .creation_rules
            .get("task")
            .expect("creation_rules.task must exist");
        assert!(!task_rules.required_body);
        assert_eq!(
            task_rules.body_sections,
            vec!["Background", "Acceptance criteria", "Exceptions"]
        );

        // Node rules — empty by default (no restrictions)
        assert!(policy.node_rules.is_empty());

        // Revise rules
        let revise = policy.revise_rules.expect("revise_rules must exist");
        assert_eq!(
            revise.allow_body_revise,
            vec![
                "draft",
                "proposed",
                "open",
                "pending",
                "designing",
                "implementing"
            ]
        );
        assert_eq!(
            revise.allow_node_revise,
            vec![
                "draft",
                "proposed",
                "under-review",
                "open",
                "pending",
                "designing",
                "implementing",
                "reviewing"
            ]
        );

        // Evidence rules
        let evidence = policy.evidence_rules.expect("evidence_rules must exist");
        assert_eq!(
            evidence.allow_evidence,
            vec![
                "draft",
                "proposed",
                "under-review",
                "open",
                "pending",
                "designing",
                "implementing",
                "reviewing",
                "closed",
                "accepted",
                "rejected",
                "deprecated"
            ]
        );
    }

    #[test]
    fn default_policy_matches_fixture() {
        let fixture =
            std::fs::read_to_string("tests/fixtures/policy_default.toml").expect("fixture exists");
        let from_const: Policy = toml::from_str(DEFAULT_POLICY).expect("DEFAULT_POLICY must parse");
        let from_fixture: Policy = toml::from_str(&fixture).expect("fixture must parse");

        // Both should produce equivalent Policy structs
        assert_eq!(from_const.guards.len(), from_fixture.guards.len());
        assert_eq!(from_const.checks.strict, from_fixture.checks.strict);
        assert_eq!(
            from_const.creation_rules.len(),
            from_fixture.creation_rules.len()
        );
        assert_eq!(from_const.node_rules.len(), from_fixture.node_rules.len());

        let const_revise = from_const.revise_rules.unwrap();
        let fixture_revise = from_fixture.revise_rules.unwrap();
        assert_eq!(
            const_revise.allow_body_revise,
            fixture_revise.allow_body_revise
        );
        assert_eq!(
            const_revise.allow_node_revise,
            fixture_revise.allow_node_revise
        );

        let const_evidence = from_const.evidence_rules.unwrap();
        let fixture_evidence = from_fixture.evidence_rules.unwrap();
        assert_eq!(
            const_evidence.allow_evidence,
            fixture_evidence.allow_evidence
        );
    }
}
