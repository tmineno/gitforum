//! Snapshot read/write at `refs/forum/threads/<id>` per SPEC-3.0 §4.
//!
//! This module owns the SPEC-3.0 §4 schema boundary. Tree assembly
//! and disassembly happens here; per-concept structs in
//! [`crate::internal::thread::ThreadSnapshot`],
//! [`crate::internal::node::NodeRecord`],
//! [`crate::internal::evidence::EvidenceFile`], and
//! [`super::link::Links`] only handle their own files.
//!
//! Phase 1: only `write_snapshot` lands here. `read_snapshot` is
//! Phase 1 step 7.

use crate::internal::error::ForumError;
use crate::internal::evidence::EvidenceFile;
use crate::internal::git_ops::GitOps;
use crate::internal::node::NodeRecord;
use crate::internal::thread::ThreadSnapshot;

use super::link::Links;

/// In-memory representation of one thread's full snapshot tree.
///
/// Maps to:
/// ```text
/// thread.toml      ← snapshot
/// body.md          ← body                (omitted if None or empty)
/// nodes/<id>.toml  ← nodes[i].record     (omitted if no nodes)
/// nodes/<id>.md    ← nodes[i].body       (omitted if empty)
/// links.toml       ← links               (omitted if empty)
/// evidence.toml    ← evidence            (omitted if empty)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadDocument {
    pub snapshot: ThreadSnapshot,
    pub body: Option<String>,
    pub nodes: Vec<NodeWithBody>,
    pub links: Links,
    pub evidence: EvidenceFile,
}

/// One node's metadata + body, paired so the writer can emit the two
/// files atomically inside `nodes/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeWithBody {
    pub record: NodeRecord,
    pub body: String,
}

impl ThreadDocument {
    /// Construct a minimal document with no body, nodes, links, or
    /// evidence. The thread.toml is required and is provided.
    pub fn new(snapshot: ThreadSnapshot) -> Self {
        Self {
            snapshot,
            body: None,
            nodes: Vec::new(),
            links: Links::default(),
            evidence: EvidenceFile::default(),
        }
    }
}

/// Write `doc` as a snapshot commit on `refs/forum/threads/<thread_id>`.
///
/// Behavior:
/// - **Create** (ref absent): writes a parentless commit and creates
///   the ref atomically. A concurrent create on the same `<id>` →
///   exactly one writer wins; loser fails with
///   [`ForumError::SnapshotWriteConflict`].
/// - **Update** (ref present): commit's parent is the current ref tip;
///   updates the ref via CAS. A stale-parent racing write fails with
///   [`ForumError::SnapshotWriteConflict`]; the ref is unchanged.
///
/// Returns the new commit's OID.
pub fn write_snapshot(
    git: &GitOps,
    thread_id: &str,
    doc: &ThreadDocument,
    message: &str,
) -> Result<String, ForumError> {
    let tree_sha = build_tree(git, doc)?;
    let refname = format!("refs/forum/threads/{thread_id}");

    let parent = git.resolve_ref(&refname)?;

    let parent_refs: Vec<&str> = parent.iter().map(String::as_str).collect();
    let commit_sha = git.commit_tree(&tree_sha, &parent_refs, message)?;

    match parent {
        None => git.create_ref(&refname, &commit_sha).map_err(|_| {
            ForumError::SnapshotWriteConflict(format!(
                "concurrent create on {refname}: ref already exists"
            ))
        })?,
        Some(old_sha) => {
            git.update_ref_cas(&refname, &commit_sha, &old_sha)
                .map_err(|_| {
                    ForumError::SnapshotWriteConflict(format!(
                        "stale parent on {refname}: expected {old_sha}, ref was updated by another writer"
                    ))
                })?
        }
    }

    Ok(commit_sha)
}

/// Assemble the SPEC-3.0 §4.2 snapshot tree from `doc`.
///
/// Optional files (body.md, links.toml, evidence.toml, nodes/) are
/// omitted when empty per SPEC-3.0 §4.2 ("MAY be absent when empty").
/// `thread.toml` is always written.
fn build_tree(git: &GitOps, doc: &ThreadDocument) -> Result<String, ForumError> {
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
    git.run_with_stdin(&["mktree"], tree_input.as_bytes())
}

/// Read the snapshot at `refs/forum/threads/<thread_id>` and assemble
/// a [`ThreadDocument`].
///
/// Errors:
/// - [`ForumError::SnapshotMissing`] — ref does not exist or its tip
///   tree lacks `thread.toml`.
/// - [`ForumError::LegacyEventChain`] — tip tree contains `event.json`
///   (an unmigrated 1.x/2.x event). Pre-flight check; runs before
///   any snapshot parse.
/// - [`ForumError::SnapshotSchemaUnsupported`] — `thread.toml`
///   `schema_version` is absent or not 3.
/// - [`ForumError::SnapshotInvalid`] — any other schema/grammar
///   failure encountered while parsing one of the files.
/// - [`ForumError::Toml`] — TOML parse error with line/column context.
pub fn read_snapshot(git: &GitOps, thread_id: &str) -> Result<ThreadDocument, ForumError> {
    let refname = format!("refs/forum/threads/{thread_id}");
    let tip = git
        .resolve_ref(&refname)?
        .ok_or_else(|| ForumError::SnapshotMissing(format!("{refname} does not exist")))?;
    read_snapshot_at(git, &tip)
}

/// Like [`read_snapshot`] but parses the tree at a specific commit
/// SHA. Used by the Phase 2 mixed-chain replay
/// (`thread::replay_thread`) to seed `ThreadState` from a snapshot
/// commit that is no longer the tip.
pub fn read_snapshot_at(git: &GitOps, tip: &str) -> Result<ThreadDocument, ForumError> {
    let tree_listing = git.run(&["ls-tree", "-r", "--name-only", tip])?;
    let paths: Vec<&str> = tree_listing.lines().collect();

    // Legacy pre-flight: an event.json blob at tip tree means this is
    // an unmigrated 1.x/2.x event chain. Reject before parsing.
    if paths.contains(&"event.json") {
        return Err(ForumError::LegacyEventChain);
    }

    if !paths.contains(&"thread.toml") {
        return Err(ForumError::SnapshotMissing(format!(
            "commit {tip} lacks thread.toml"
        )));
    }

    let snapshot = ThreadSnapshot::from_toml(&git.show_file(tip, "thread.toml")?)?;

    let body = if paths.contains(&"body.md") {
        let raw = git.show_file_bytes(tip, "body.md")?;
        Some(
            String::from_utf8(raw)
                .map_err(|e| ForumError::SnapshotInvalid(format!("body.md utf-8: {e}")))?,
        )
    } else {
        None
    };

    let mut nodes: Vec<NodeWithBody> = Vec::new();
    let mut node_ids: Vec<String> = paths
        .iter()
        .filter_map(|p| {
            p.strip_prefix("nodes/")
                .and_then(|s| s.strip_suffix(".toml"))
        })
        .map(String::from)
        .collect();
    node_ids.sort();
    for nid in node_ids {
        let toml_path = format!("nodes/{nid}.toml");
        let md_path = format!("nodes/{nid}.md");
        let record = NodeRecord::from_toml(&git.show_file(tip, &toml_path)?)?;
        let body = if paths.contains(&md_path.as_str()) {
            let raw = git.show_file_bytes(tip, &md_path)?;
            String::from_utf8(raw)
                .map_err(|e| ForumError::SnapshotInvalid(format!("nodes/{nid}.md utf-8: {e}")))?
        } else {
            String::new()
        };
        nodes.push(NodeWithBody { record, body });
    }

    let links = if paths.contains(&"links.toml") {
        Links::from_toml(&git.show_file(tip, "links.toml")?)?
    } else {
        Links::default()
    };

    let evidence = if paths.contains(&"evidence.toml") {
        EvidenceFile::from_toml(&git.show_file(tip, "evidence.toml")?)?
    } else {
        EvidenceFile::default()
    };

    Ok(ThreadDocument {
        snapshot,
        body,
        nodes,
        links,
        evidence,
    })
}
