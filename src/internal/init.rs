use std::fs;

use super::config::RepoPaths;
use super::error::ForumResult;
use super::git_ops::GitOps;

const DEFAULT_POLICY: &str = r#"# git-forum default policy (SPEC-3.0 §3.2 / §3.3)
#
# Policy is keyed by category. Built-in categories (SPEC-3.0 §3.1):
#   rfc:  draft → open → review → {done, rejected} → deprecated
#         (also: draft|open → withdrawn)
#   task: open ⇄ working ⇄ review → {done, rejected} → deprecated
#
# State names are SPEC-3.0 canonical: draft, open, working, review, done,
# rejected, withdrawn, deprecated. Legacy 2.x policy shapes
# ([[guards]] requires=..., kind/lifecycle/facet-scoped creation_rules.*,
# node_rules / revise_rules / evidence_rules tables, the
# `one_human_approval` and `at_least_one_summary` rule names) are
# rejected at load time with a migration hint pointing at
# `git forum migrate --to 3.0`.
#
# Available guard rules (SPEC-3.0 §3.2):
#   no_open_objections   — all objection nodes must be resolved/retracted
#   no_open_actions      — all action nodes must be resolved/retracted
#   one_approval         — at least one non-retracted approval node
#                          (any actor type)
#   has_commit_evidence  — thread must have commit-type evidence

# ═══════════════════════════════════════════════════════════════════════
# 1. GUARDS (SPEC-3.0 §3.2)
# ═══════════════════════════════════════════════════════════════════════

[categories.rfc.guards]
"review->done" = ["one_approval", "no_open_objections"]

[categories.task.guards]
"open->done" = ["no_open_actions"]
"working->done" = ["no_open_actions"]
"review->done" = ["no_open_actions"]

# Uncomment to require commit evidence before closing task threads:
# [categories.task.guards]
# "review->done" = ["no_open_actions", "has_commit_evidence"]

# ═══════════════════════════════════════════════════════════════════════
# 2. OPERATION CHECKS (SPEC-3.0 §3.3)
# ═══════════════════════════════════════════════════════════════════════
# Severity levels:
#   strict = false (default) — violations are WARNINGS; use --force to bypass
#   strict = true            — violations are ERRORS; operation is blocked

[checks]
strict = false

# Creation rules: what is required when creating a new thread per category.
[categories.rfc.creation]
required_body = true
body_sections = ["Goal", "Non-goals", "Context", "Proposal"]

[categories.task.creation]
required_body = false
body_sections = ["Background", "Acceptance criteria", "Exceptions"]

# Allowed node types per status. Empty/missing = no restriction.
# Uncomment to restrict node types in terminal states:
# [categories.rfc.allowed_node_types]
# "done"     = []
# "rejected" = []
#
# [categories.task.allowed_node_types]
# "done"     = []
# "rejected" = []

# Revise rules: in which states body/node revision is allowed per category.
[categories.rfc.revise]
allow_body_revise = ["draft", "open"]
allow_node_revise = ["draft", "open", "review"]

[categories.task.revise]
allow_body_revise = ["open", "working"]
allow_node_revise = ["open", "working", "review"]

# Evidence rules: in which states evidence can be attached per category.
[categories.rfc.evidence]
allow_evidence = ["draft", "open", "review", "done", "rejected", "deprecated"]

[categories.task.evidence]
allow_evidence = ["open", "working", "review", "done", "rejected", "deprecated"]
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
    use crate::internal::policy::{GuardRule, Policy};

    #[test]
    fn default_policy_parses_under_v3_parser() {
        // The DEFAULT_POLICY const must be loadable through the strict
        // SPEC-3.0 parser without warnings or rewrites — i.e. it must
        // already be in the §3.2/§3.3 category-table form.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.toml");
        std::fs::write(&path, DEFAULT_POLICY).unwrap();
        let policy = Policy::load(&path).expect("DEFAULT_POLICY must parse under v3 parser");

        // Guards: rfc has review->done, task has three transitions.
        let rfc = policy.category("rfc").expect("rfc category present");
        assert_eq!(
            rfc.guards.get("review->done").map(|v| v.as_slice()),
            Some([GuardRule::OneApproval, GuardRule::NoOpenObjections].as_slice())
        );
        let task = policy.category("task").expect("task category present");
        assert!(task.guards.contains_key("review->done"));
        assert!(task.guards.contains_key("open->done"));
        assert!(task.guards.contains_key("working->done"));

        // Creation rules per category.
        let rfc_creation = rfc.creation.as_ref().expect("rfc creation");
        assert!(rfc_creation.required_body);
        assert_eq!(
            rfc_creation.body_sections,
            vec!["Goal", "Non-goals", "Context", "Proposal"]
        );
        let task_creation = task.creation.as_ref().expect("task creation");
        assert!(!task_creation.required_body);
        assert_eq!(
            task_creation.body_sections,
            vec!["Background", "Acceptance criteria", "Exceptions"]
        );

        // Revise rules per category, intersected with each category's
        // statuses (rfc has no `working`, task has no `draft`).
        let rfc_revise = rfc.revise.as_ref().expect("rfc revise");
        assert_eq!(rfc_revise.allow_body_revise, vec!["draft", "open"]);
        assert_eq!(
            rfc_revise.allow_node_revise,
            vec!["draft", "open", "review"]
        );
        let task_revise = task.revise.as_ref().expect("task revise");
        assert_eq!(task_revise.allow_body_revise, vec!["open", "working"]);

        // Checks default: strict = false.
        assert!(!policy.checks.strict);
    }

    #[test]
    fn default_policy_matches_fixture() {
        let fixture_dir = tempfile::tempdir().unwrap();
        let fixture_path = fixture_dir.path().join("fixture.toml");
        std::fs::copy("tests/fixtures/policy_default.toml", &fixture_path).unwrap();
        let const_dir = tempfile::tempdir().unwrap();
        let const_path = const_dir.path().join("default.toml");
        std::fs::write(&const_path, DEFAULT_POLICY).unwrap();

        let from_const = Policy::load(&const_path).expect("DEFAULT_POLICY parses");
        let from_fixture = Policy::load(&fixture_path).expect("fixture parses");

        assert_eq!(from_const.checks.strict, from_fixture.checks.strict);
        assert_eq!(
            from_const.categories.len(),
            from_fixture.categories.len(),
            "category count must match"
        );
        for cat_name in ["rfc", "task"] {
            let c = from_const.category(cat_name).expect("const has category");
            let f = from_fixture
                .category(cat_name)
                .expect("fixture has category");
            assert_eq!(c.guards.len(), f.guards.len(), "{cat_name} guards count");
            assert_eq!(
                c.creation.as_ref().map(|x| x.required_body),
                f.creation.as_ref().map(|x| x.required_body),
                "{cat_name} creation.required_body"
            );
        }
    }
}
