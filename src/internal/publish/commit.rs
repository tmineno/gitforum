//! Build and write parentless single-commit snapshots into
//! `refs/forum/published/*` per RFC `fls856j3` §2.
//!
//! The published commit shape:
//!
//! - **No `-p`** — the commit is parentless. `git log
//!   refs/forum/published/<id>` shows only the current tree.
//! - **Operator's normal git config** for author/committer/dates and
//!   signing — no synthetic identity, no pinned timestamps. Real
//!   attribution of who pushed and when is recorded.
//! - **Idempotency by tree, not commit SHA.** The publisher compares
//!   the recomputed published *tree* SHA against the tree pointed at
//!   by the current `refs/forum/published/<id>` and skips the write
//!   when they match.
//!
//! Tree shape is the SPEC-3.0 §4.2 layout minus `legacy/` (published
//! refs are derivative — the legacy archive is an authoritative-only
//! concern).

use crate::internal::error::{ForumError, ForumResult};
use crate::internal::git_ops::GitOps;
use crate::internal::refs;
use crate::internal::snapshot::ThreadDocument;

/// Outcome of an attempted write into the published namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The published ref was newly created at `commit_sha`.
    Created { commit_sha: String },
    /// The published ref was force-updated to `commit_sha`.
    Updated { commit_sha: String },
    /// The recomputed tree matched the current tree on the published
    /// ref, so no write happened (RFC §2 tree-equivalence
    /// idempotency).
    Skipped,
}

/// Build the published tree for `doc` and return its SHA.
///
/// Preconditions: `doc` is the post-exclusion `ThreadDocument`
/// (callers should have already applied `publish::exclusion::apply`).
/// Postconditions: returns a valid tree OID containing
/// `thread.toml`, plus optional `body.md`, `nodes/`, `links.toml`,
/// `evidence.toml`. `legacy/` is never included.
/// Failure modes: any I/O error from `hash-object` or `mktree`.
/// Side effects: writes blob/tree objects into the local ODB.
pub fn build_published_tree(git: &GitOps, doc: &ThreadDocument) -> ForumResult<String> {
    let mut entries: Vec<String> = Vec::new();

    // Required: thread.toml
    let thread_toml = doc.snapshot.to_toml()?;
    let sha = git.hash_object(thread_toml.as_bytes())?;
    entries.push(format!("100644 blob {sha}\tthread.toml"));

    // Optional: body.md
    if let Some(body) = &doc.body {
        if !body.is_empty() {
            let sha = git.hash_object(body.as_bytes())?;
            entries.push(format!("100644 blob {sha}\tbody.md"));
        }
    }

    // Optional: nodes/ subtree
    if !doc.nodes.is_empty() {
        let mut node_entries: Vec<String> = Vec::new();
        for node in &doc.nodes {
            let toml = node.record.to_toml()?;
            let toml_sha = git.hash_object(toml.as_bytes())?;
            node_entries.push(format!("100644 blob {}\t{}.toml", toml_sha, node.record.id));
            if !node.body.is_empty() {
                let body_sha = git.hash_object(node.body.as_bytes())?;
                node_entries.push(format!("100644 blob {}\t{}.md", body_sha, node.record.id));
            }
        }
        let subtree_input = format!("{}\n", node_entries.join("\n"));
        let nodes_tree_sha = git.run_with_stdin(&["mktree"], subtree_input.as_bytes())?;
        entries.push(format!("040000 tree {nodes_tree_sha}\tnodes"));
    }

    // Optional: links.toml
    if !doc.links.is_empty() {
        let toml = doc.links.to_toml()?;
        let sha = git.hash_object(toml.as_bytes())?;
        entries.push(format!("100644 blob {sha}\tlinks.toml"));
    }

    // Optional: evidence.toml
    if !doc.evidence.is_empty() {
        let toml = doc.evidence.to_toml()?;
        let sha = git.hash_object(toml.as_bytes())?;
        entries.push(format!("100644 blob {sha}\tevidence.toml"));
    }

    let tree_input = format!("{}\n", entries.join("\n"));
    let tree_sha = git.run_with_stdin(&["mktree"], tree_input.as_bytes())?;
    Ok(tree_sha)
}

/// Read the tree SHA pointed at by `refs/forum/published/<thread_id>`.
/// Returns `None` when the ref does not exist.
pub fn current_published_tree(git: &GitOps, thread_id: &str) -> ForumResult<Option<String>> {
    let refname = refs::published_ref(thread_id);
    let commit = match git.resolve_ref(&refname)? {
        Some(c) => c,
        None => return Ok(None),
    };
    let tree = git
        .run(&["rev-parse", &format!("{commit}^{{tree}}")])?
        .trim()
        .to_string();
    Ok(Some(tree))
}

/// Write `doc` (already post-exclusion) into the published namespace
/// for `thread_id`.
///
/// The flow:
///
/// 1. Build the published tree from `doc`.
/// 2. Compare with the tree on the current `refs/forum/published/<id>`
///    (if any). If the trees match, return [`WriteOutcome::Skipped`]
///    (RFC §2 tree-equivalence idempotency).
/// 3. Otherwise build a parentless commit (`commit-tree TREE -m ...`,
///    no `-p`) and force-update the ref.
///
/// Preconditions: `doc.snapshot.visibility == Visibility::Public`
/// (caller must enforce; this function does not).
/// Postconditions: on `Created`/`Updated`, the local
/// `refs/forum/published/<thread_id>` points at a parentless commit
/// whose tree equals the rebuild.
/// Failure modes: any subprocess error from git.
/// Side effects: writes one blob/tree set and (on non-skip) one
/// commit object plus one ref update.
pub fn write_published(
    git: &GitOps,
    thread_id: &str,
    doc: &ThreadDocument,
) -> ForumResult<WriteOutcome> {
    let new_tree = build_published_tree(git, doc)?;
    let current = current_published_tree(git, thread_id)?;

    if current.as_deref() == Some(new_tree.as_str()) {
        return Ok(WriteOutcome::Skipped);
    }

    let message = format!("published snapshot of {thread_id}");
    let commit_sha = git.commit_tree(&new_tree, &[], &message)?;
    let refname = refs::published_ref(thread_id);
    git.update_ref(&refname, &commit_sha)?;

    if current.is_some() {
        Ok(WriteOutcome::Updated { commit_sha })
    } else {
        Ok(WriteOutcome::Created { commit_sha })
    }
}

/// Delete the local `refs/forum/published/<thread_id>` ref. No-op
/// when the ref already does not exist. Used by the withdrawal flow
/// (RFC §7) **after** the remote has accepted the deletion — the
/// preserve-then-retry rule keeps local state consistent with the
/// last successful push.
pub fn delete_published(git: &GitOps, thread_id: &str) -> ForumResult<bool> {
    let refname = refs::published_ref(thread_id);
    if git.resolve_ref(&refname)?.is_none() {
        return Ok(false);
    }
    git.delete_ref(&refname).map_err(|e| match e {
        ForumError::Git(msg) => ForumError::Git(format!("delete-ref {refname}: {msg}")),
        other => other,
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::git_ops::GitOps;
    use chrono::Utc;
    use tempfile::TempDir;

    use crate::internal::evidence::EvidenceFile;
    use crate::internal::snapshot::link::Links;
    use crate::internal::thread::{ThreadSnapshot, Visibility};

    fn fresh_repo() -> (TempDir, GitOps) {
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        // Init a real git repo so commit-tree etc. work.
        git.run(&["init", "-q"]).unwrap();
        git.run(&["config", "user.name", "tester"]).unwrap();
        git.run(&["config", "user.email", "t@example"]).unwrap();
        (dir, git)
    }

    fn epoch() -> chrono::DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn doc(id: &str, body: Option<&str>) -> ThreadDocument {
        ThreadDocument {
            snapshot: ThreadSnapshot {
                schema_version: 3,
                id: id.into(),
                title: "Pub".into(),
                category: "rfc".into(),
                status: "draft".into(),
                tags: vec![],
                created_at: epoch(),
                created_by: "human/alice".into(),
                updated_at: epoch(),
                updated_by: "human/alice".into(),
                branch: None,
                supersedes: vec![],
                visibility: Visibility::Public,
            },
            body: body.map(String::from),
            nodes: vec![],
            links: Links { entries: vec![] },
            evidence: EvidenceFile { entries: vec![] },
        }
    }

    #[test]
    fn build_published_tree_omits_legacy() {
        let (_dir, git) = fresh_repo();
        let d = doc("pub00000", Some("hello\n"));
        let tree_sha = build_published_tree(&git, &d).unwrap();

        let listing = git.run(&["ls-tree", "--name-only", &tree_sha]).unwrap();
        // No legacy/ entry on a published tree.
        assert!(!listing.contains("legacy"), "tree listing was: {listing}");
        assert!(listing.contains("thread.toml"));
        assert!(listing.contains("body.md"));
    }

    #[test]
    fn write_published_creates_then_skips_then_updates() {
        let (_dir, git) = fresh_repo();
        let d = doc("pub00000", Some("first\n"));

        let r1 = write_published(&git, "pub00000", &d).unwrap();
        assert!(matches!(r1, WriteOutcome::Created { .. }));

        // Second write of identical content → skip (tree match).
        let r2 = write_published(&git, "pub00000", &d).unwrap();
        assert_eq!(r2, WriteOutcome::Skipped);

        // Mutating the body changes the tree → update.
        let d2 = doc("pub00000", Some("second\n"));
        let r3 = write_published(&git, "pub00000", &d2).unwrap();
        assert!(matches!(r3, WriteOutcome::Updated { .. }));
    }

    #[test]
    fn published_commit_is_parentless() {
        let (_dir, git) = fresh_repo();
        let d = doc("pub00000", Some("first\n"));
        let r1 = write_published(&git, "pub00000", &d).unwrap();
        let sha = match r1 {
            WriteOutcome::Created { commit_sha } => commit_sha,
            other => panic!("expected Created, got {other:?}"),
        };
        // git rev-list --count should be 1 for a parentless commit.
        let count = git.run(&["rev-list", "--count", &sha]).unwrap();
        assert_eq!(count.trim(), "1");
        // git rev-parse <sha>^ should fail for a parentless commit.
        let parent = git.run(&["rev-parse", &format!("{sha}^")]);
        assert!(
            parent.is_err(),
            "parentless commit must have no parent; got {parent:?}"
        );
    }

    #[test]
    fn updates_reset_to_parentless() {
        // RFC §2: every published commit is parentless, even on
        // updates. The next publish does NOT chain the new commit
        // on the previous one.
        let (_dir, git) = fresh_repo();
        let d1 = doc("pub00000", Some("v1"));
        let r1 = write_published(&git, "pub00000", &d1).unwrap();
        let sha1 = match r1 {
            WriteOutcome::Created { commit_sha } => commit_sha,
            o => panic!("{o:?}"),
        };

        let d2 = doc("pub00000", Some("v2"));
        let r2 = write_published(&git, "pub00000", &d2).unwrap();
        let sha2 = match r2 {
            WriteOutcome::Updated { commit_sha } => commit_sha,
            o => panic!("{o:?}"),
        };

        assert_ne!(sha1, sha2);
        // sha2 is parentless — its rev-list count is 1, not 2.
        let count = git.run(&["rev-list", "--count", &sha2]).unwrap();
        assert_eq!(count.trim(), "1");
    }

    #[test]
    fn tree_equivalence_across_clones_in_a_single_repo() {
        // Two distinct ThreadDocument instances with identical
        // content produce the same tree SHA. This is the property
        // RFC §4.5 calls "tree-equivalence" — even when commit
        // metadata differs, the tree is content-addressed.
        let (_dir, git) = fresh_repo();
        let a = doc("pub00000", Some("body bytes\n"));
        let b = doc("pub00000", Some("body bytes\n"));
        let ta = build_published_tree(&git, &a).unwrap();
        let tb = build_published_tree(&git, &b).unwrap();
        assert_eq!(ta, tb);
    }

    #[test]
    fn delete_published_when_present_and_absent() {
        let (_dir, git) = fresh_repo();
        // Absent: returns false, no error.
        assert!(!delete_published(&git, "absent00").unwrap());
        // Present: write, then delete.
        let d = doc("pub00000", Some("body"));
        write_published(&git, "pub00000", &d).unwrap();
        assert!(delete_published(&git, "pub00000").unwrap());
        // Idempotent: second delete is a no-op.
        assert!(!delete_published(&git, "pub00000").unwrap());
    }
}
