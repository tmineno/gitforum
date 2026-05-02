use std::fs;

use super::config::RepoPaths;
use super::error::ForumResult;
use super::git_ops::GitOps;

const DEFAULT_POLICY: &str = r#"# git-forum default policy
#
# This file has two sections:
#   1. Transition guards — conditions that must be met before a state change.
#   2. Operation checks  — rules about what is allowed in each state.
#
# Per SPEC-2.0 §3.1, all state names are 2.0 canonical: draft, open,
# working, review, done, rejected, withdrawn, deprecated. Legacy 1.x
# names (proposed, under-review, accepted, closed, pending, designing,
# implementing, reviewing) are accepted at load time but produce a
# migration warning; they are normalized internally.
#
# Per-lifecycle reachable states (SPEC-2.0 §3.1.1):
#   proposal  (RFC):       draft, open, review, done, rejected, withdrawn, deprecated
#   execution (issue/task): open, working, review, done, rejected, withdrawn
#   record    (DEC):       open, done, rejected, withdrawn, deprecated

# ═══════════════════════════════════════════════════════════════════════
# 1. TRANSITION GUARDS
# ═══════════════════════════════════════════════════════════════════════
# Each [[guards]] block defines conditions ("requires") that must ALL pass
# before the transition in "on" is allowed.
#
# A guard violation is always an error — the transition is blocked.
# This is different from operation checks (below), which can be warnings.
#
# Lifecycle-scoped guards use the SPEC-2.0 §7.1 facet predicate syntax:
#   on = "lifecycle=proposal : review->done"   — only matches RFC threads
#   on = "review->done"                         — matches all lifecycles
#   on = "lifecycle=execution AND tag=bug : open->done"
#
# When both a scoped and unscoped guard match, both apply (union semantics).
#
# Available guard rules:
#   no_open_objections   — all objection nodes must be resolved/retracted
#   no_open_actions      — all action nodes must be resolved/retracted
#   one_human_approval   — at least one recorded human/… actor approval required
#   has_commit_evidence  — thread must have commit-type evidence attached

[[guards]]
on = "lifecycle=proposal : review->done"
requires = ["one_human_approval", "no_open_objections"]

[[guards]]
on = "lifecycle=execution : open->done"
requires = ["no_open_actions"]

[[guards]]
on = "lifecycle=execution : review->done"
requires = ["no_open_actions"]

[[guards]]
on = "lifecycle=record : open->done"
requires = ["no_open_objections"]

# Uncomment to require commit evidence before closing execution threads:
# [[guards]]
# on = "lifecycle=execution : open->done"
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
# State names in operation checks are 2.0 canonical (above). The runtime
# tolerates 1.x names on either side of the comparison so legacy policies
# keep working, but defaults use the canonical vocabulary.

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
# No restrictions by default — all node types allowed in all states.
# Uncomment to restrict node types in terminal states:
# [node_rules]
# "done" = []
# "rejected" = []
# "deprecated" = []
# "withdrawn" = []

# Revise rules: in which states body/node revision is allowed.
# Body revision is intentionally narrower than node revision — once a
# thread is under formal review (or working), the body should be stable.
[revise_rules]
allow_body_revise = ["draft", "open", "working"]
allow_node_revise = ["draft", "open", "working", "review"]

# Evidence rules: in which states evidence can be attached.
# An empty list (or omitting [evidence_rules] entirely) means evidence
# is allowed in every state. Listed here for documentation; users can
# narrow it if needed.
[evidence_rules]
allow_evidence = ["draft", "open", "working", "review", "done", "rejected", "deprecated"]
"#;

const DEFAULT_ACTORS: &str = r#"# git-forum actors
# Register human and AI actors here.
#
# [[actors]]
# id = "human/alice"
# kind = "human"
# display_name = "Alice"
"#;

// Template bodies are embedded from the tracked .forum/templates/*.md
// files (single source of truth, ADR-007). The same physical file backs
// both git-forum's own forum and the seed written to user repos by
// `git forum init`.
const TEMPLATE_ISSUE: &str = include_str!("../../.forum/templates/issue.md");
const TEMPLATE_RFC: &str = include_str!("../../.forum/templates/rfc.md");
const TEMPLATE_DEC: &str = include_str!("../../.forum/templates/dec.md");
const TEMPLATE_TASK: &str = include_str!("../../.forum/templates/task.md");

/// Full first-time init of `.forum/` and `.git/forum/` structure.
///
/// Writes shared config and templates under `.forum/`, plus the
/// per-worktree state under `.git/forum/`. Idempotent: skips files that
/// already exist. Used by `git forum init`.
///
/// For per-worktree first-touch (e.g. the post-checkout hook on a new
/// worktree), use [`init_forum_local`] instead — `worktree-init` must not
/// re-seed shared `.forum/` content (ADR-007).
pub fn init_forum(paths: &RepoPaths) -> ForumResult<()> {
    let templates_dir = paths.dot_forum.join("templates");
    fs::create_dir_all(&templates_dir)?;

    write_if_missing(&paths.dot_forum.join("policy.toml"), DEFAULT_POLICY)?;
    write_if_missing(&paths.dot_forum.join("actors.toml"), DEFAULT_ACTORS)?;
    write_if_missing(&templates_dir.join("issue.md"), TEMPLATE_ISSUE)?;
    write_if_missing(&templates_dir.join("rfc.md"), TEMPLATE_RFC)?;
    write_if_missing(&templates_dir.join("dec.md"), TEMPLATE_DEC)?;
    write_if_missing(&templates_dir.join("task.md"), TEMPLATE_TASK)?;

    init_forum_local(paths)
}

/// Per-worktree init: writes only `.git/forum/` content and the local
/// git alias. Does not touch `.forum/`.
///
/// Used by the `worktree-init` post-checkout hook so a fresh worktree
/// gets its per-clone state without overwriting or seeding any tracked
/// shared content (ADR-007).
pub fn init_forum_local(paths: &RepoPaths) -> ForumResult<()> {
    fs::create_dir_all(paths.git_forum.join("logs"))?;

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

/// The fetch refspec that maps remote forum refs into the local namespace.
///
/// The `+` prefix allows force-updates (forum refs are amended by design).
pub const FORUM_REFSPEC: &str = "+refs/forum/*:refs/forum/*";

/// Check whether a remote already has the forum fetch refspec configured.
pub fn has_forum_refspec(git: &GitOps, remote: &str) -> ForumResult<bool> {
    let key = format!("remote.{remote}.fetch");
    match git.run(&["config", "--get-all", &key]) {
        Ok(output) => Ok(output.lines().any(|line| line.trim() == FORUM_REFSPEC)),
        Err(_) => Ok(false),
    }
}

/// For every configured remote, add the forum fetch refspec if not already present.
///
/// Idempotent: skips remotes that already have the refspec.
/// Returns the list of remote names that were modified.
pub fn ensure_forum_refspecs(git: &GitOps) -> ForumResult<Vec<String>> {
    let remotes_output = match git.run(&["remote"]) {
        Ok(o) => o,
        Err(_) => return Ok(vec![]),
    };
    let mut modified = Vec::new();
    for remote in remotes_output.lines() {
        let remote = remote.trim();
        if remote.is_empty() {
            continue;
        }
        if !has_forum_refspec(git, remote)? {
            git.run(&[
                "config",
                "--add",
                &format!("remote.{remote}.fetch"),
                FORUM_REFSPEC,
            ])?;
            modified.push(remote.to_string());
        }
    }
    Ok(modified)
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

        // Guards (2.0 canonical with lifecycle facets, per @ltojzq9l).
        assert_eq!(policy.guards.len(), 4);
        assert_eq!(policy.guards[0].on, "lifecycle=proposal : review->done");
        assert_eq!(policy.guards[1].on, "lifecycle=execution : open->done");
        assert_eq!(policy.guards[2].on, "lifecycle=execution : review->done");
        assert_eq!(policy.guards[3].on, "lifecycle=record : open->done");

        // Checks config
        assert!(!policy.checks.strict);

        // Creation rules — RFC (legacy `rfc` key, base section before
        // load-time auto-translation).
        let rfc_rules = policy
            .creation_rules
            .get("rfc")
            .expect("creation_rules.rfc must exist");
        assert!(rfc_rules.base.required_body);
        assert_eq!(
            rfc_rules.base.body_sections,
            vec!["Goal", "Non-goals", "Context", "Proposal"]
        );

        // Creation rules — issue
        let issue_rules = policy
            .creation_rules
            .get("issue")
            .expect("creation_rules.issue must exist");
        assert!(!issue_rules.base.required_body);
        assert!(issue_rules.base.body_sections.is_empty());

        // Creation rules — dec
        let dec_rules = policy
            .creation_rules
            .get("dec")
            .expect("creation_rules.dec must exist");
        assert!(dec_rules.base.required_body);
        assert_eq!(
            dec_rules.base.body_sections,
            vec!["Context", "Decision", "Rationale", "Impact"]
        );

        // Creation rules — task
        let task_rules = policy
            .creation_rules
            .get("task")
            .expect("creation_rules.task must exist");
        assert!(!task_rules.base.required_body);
        assert_eq!(
            task_rules.base.body_sections,
            vec!["Background", "Acceptance criteria", "Exceptions"]
        );

        // Node rules — empty by default (no restrictions)
        assert!(policy.node_rules.is_empty());

        // Revise rules (2.0 canonical, per @ltojzq9l).
        let revise = policy.revise_rules.expect("revise_rules must exist");
        assert_eq!(revise.allow_body_revise, vec!["draft", "open", "working"]);
        assert_eq!(
            revise.allow_node_revise,
            vec!["draft", "open", "working", "review"]
        );

        // Evidence rules (2.0 canonical).
        let evidence = policy.evidence_rules.expect("evidence_rules must exist");
        assert_eq!(
            evidence.allow_evidence,
            vec![
                "draft",
                "open",
                "working",
                "review",
                "done",
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
