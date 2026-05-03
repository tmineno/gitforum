//! `git forum brief <THREAD> [--json]` — read-only single-thread digest.
//!
//! See RFC-5wf2v8hv (split from RFC-0022) and CORE-VALUE.md "Advisories".
//!
//! Hard boundaries (enforced by API shape, not runtime checks):
//!
//! - Reads only the named thread's events. The optional incoming-link count
//!   comes from the SQLite reverse-link index, which is a derived metadata
//!   table — no other thread's ref is opened to render `brief`.
//! - Never reads linked threads' bodies, titles, or states. Outgoing-link
//!   summaries report **counts grouped by relation only**.
//! - Never appends an event.
//! - No flag suggests cross-thread analysis (no `--tree`, no
//!   `--with-parent`, no `--show-blockers`).
//!
//! Phase 2 slot 7h (RFC `7ymtc4b2`): the `Brief` arm body relocates
//! from `main.rs` to [`run`] in this module, and
//! [`read_incoming_link_counts`] moves here. Per ADR-011 Decision 6
//! the SQLite reverse-link index is on the Phase 4 DELETE list, so
//! `read_incoming_link_counts` returns the zero-counts default until
//! a future slot adds a tree-scan fallback.

use std::collections::BTreeMap;

use serde::Serialize;

use super::super::config;
use super::super::error::ForumError;
use super::super::event::NodeType;
use super::super::node::Node;
use super::super::thread::{self, ThreadState};
use super::context::Context;
use super::shared::resolve_tid;

/// Tally of incoming `--rel <X>` link counts grouped by relation, sourced from
/// the SQLite reverse-link index.
///
/// Held as a separate input so `brief` rendering can be unit-tested without a
/// SQLite connection, and so the renderer never reads linked-thread state.
#[derive(Debug, Clone, Default)]
pub struct IncomingLinkCounts {
    /// Map from relation name to count. Empty when the index is unavailable
    /// or has no incoming links for the thread.
    pub by_rel: BTreeMap<String, usize>,
}

impl IncomingLinkCounts {
    pub fn total(&self) -> usize {
        self.by_rel.values().sum()
    }
}

/// Stable v1 schema for `--json` output.
///
/// Field set is fixed; new fields may be added (additive evolution only) per
/// RFC-5wf2v8hv. Field names and types must not change without a SPEC update.
#[derive(Debug, Clone, Serialize)]
pub struct BriefJson {
    pub id: String,
    pub title: String,
    pub lifecycle: String,
    pub tags: Vec<String>,
    pub status: String,
    pub created_at: String,
    pub created_by: String,
    pub branch: Option<String>,
    pub links_in: Vec<LinkCount>,
    pub links_out: Vec<LinkCount>,
    pub node_counts: BTreeMap<String, usize>,
    pub open_objections: usize,
    pub open_actions: usize,
    pub evidence_count: usize,
    pub latest_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkCount {
    pub rel: String,
    pub count: usize,
}

/// Render the plaintext digest for `git forum brief <THREAD>`.
///
/// Output shape matches the example in RFC-5wf2v8hv body. Deterministic given
/// deterministic input.
pub fn render_plaintext(state: &ThreadState, incoming: &IncomingLinkCounts) -> String {
    let lifecycle = state.lifecycle.as_str();
    let tags_disp = if state.tags.is_empty() {
        "-".to_string()
    } else {
        state.tags.join(", ")
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "{}  {}/{}   {}",
        state.id, lifecycle, state.status, tags_disp
    ));
    lines.push(format!("title:    {}", state.title));
    lines.push(format!(
        "created:  {} by {}",
        state.created_at.format("%Y-%m-%d"),
        state.created_by
    ));
    lines.push(format!(
        "branch:   {}",
        state.branch.as_deref().unwrap_or("-")
    ));

    let in_summary = format_link_summary(&counts_to_pairs(&incoming.by_rel));
    let out_pairs = group_links_by_rel(&state.links);
    let out_summary = format_link_summary(&out_pairs);
    lines.push(format!("links:    in={in_summary}, out={out_summary}"));

    let canonical = canonical_node_counts(&state.nodes);
    let total_nodes = state.nodes.len();
    let nodes_breakdown = format_node_breakdown(&canonical);
    let open_obj = state.open_objections().len();
    let open_act = state.open_actions().len();
    lines.push(format!(
        "nodes:    {total_nodes} ({nodes_breakdown}, {open_obj} open objections, {open_act} open actions)"
    ));

    let evidence_count = state.evidence_items.len();
    let evidence_breakdown = format_evidence_breakdown(state);
    if evidence_count > 0 {
        lines.push(format!("evidence: {evidence_count} ({evidence_breakdown})"));
    } else {
        lines.push("evidence: 0".to_string());
    }

    let summary_line = state
        .latest_summary()
        .map(|s| format!("\"{}\"", single_line(&s.body, 80)))
        .unwrap_or_else(|| "-".to_string());
    lines.push(format!("summary:  {summary_line}"));

    lines.push(String::new());
    lines.join("\n")
}

/// Build the v1 JSON payload for `git forum brief <THREAD> --json`.
pub fn build_json(state: &ThreadState, incoming: &IncomingLinkCounts) -> BriefJson {
    BriefJson {
        id: state.id.clone(),
        title: state.title.clone(),
        lifecycle: state.lifecycle.as_str().to_string(),
        tags: state.tags.clone(),
        status: state.status.to_string(),
        created_at: state.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        created_by: state.created_by.clone(),
        branch: state.branch.clone(),
        links_in: incoming
            .by_rel
            .iter()
            .map(|(rel, count)| LinkCount {
                rel: rel.clone(),
                count: *count,
            })
            .collect(),
        links_out: group_links_by_rel(&state.links)
            .into_iter()
            .map(|(rel, count)| LinkCount { rel, count })
            .collect(),
        node_counts: canonical_node_counts(&state.nodes),
        open_objections: state.open_objections().len(),
        open_actions: state.open_actions().len(),
        evidence_count: state.evidence_items.len(),
        latest_summary: state.latest_summary().map(|s| s.body.clone()),
    }
}

fn group_links_by_rel(links: &[super::super::thread::ThreadLink]) -> Vec<(String, usize)> {
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for l in links {
        *acc.entry(l.rel.clone()).or_default() += 1;
    }
    counts_to_pairs(&acc)
}

fn counts_to_pairs(map: &BTreeMap<String, usize>) -> Vec<(String, usize)> {
    map.iter().map(|(k, v)| (k.clone(), *v)).collect()
}

fn format_link_summary(pairs: &[(String, usize)]) -> String {
    if pairs.is_empty() {
        return "0".to_string();
    }
    let total: usize = pairs.iter().map(|(_, c)| *c).sum();
    if pairs.len() == 1 {
        // Compact form matches RFC-5wf2v8hv example: "in=2 implements".
        return format!("{total} {}", pairs[0].0);
    }
    // Multi-relation: spell out per-relation counts so the total isn't ambiguous.
    let parts: Vec<String> = pairs.iter().map(|(rel, c)| format!("{c} {rel}")).collect();
    format!("{total} ({})", parts.join(", "))
}

fn canonical_node_counts(nodes: &[Node]) -> BTreeMap<String, usize> {
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for n in nodes {
        let label = match n.node_type.canonical() {
            NodeType::Comment => "comment",
            NodeType::Approval => "approval",
            NodeType::Objection => "objection",
            NodeType::Action => "action",
            // canonical() never returns the legacy variants below; included
            // for exhaustiveness to keep this stable as new variants are added.
            other => other_to_label(other),
        };
        *acc.entry(label.to_string()).or_default() += 1;
    }
    acc
}

fn other_to_label(nt: NodeType) -> &'static str {
    match nt {
        NodeType::Claim => "claim",
        NodeType::Question => "question",
        NodeType::Evidence => "evidence",
        NodeType::Summary => "summary",
        NodeType::Risk => "risk",
        NodeType::Review => "review",
        NodeType::Alternative => "alternative",
        NodeType::Assumption => "assumption",
        _ => "other",
    }
}

fn format_node_breakdown(counts: &BTreeMap<String, usize>) -> String {
    if counts.is_empty() {
        return "0 comment".to_string();
    }
    counts
        .iter()
        .map(|(k, v)| format!("{v} {k}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_evidence_breakdown(state: &ThreadState) -> String {
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for ev in &state.evidence_items {
        *acc.entry(ev.kind.to_string()).or_default() += 1;
    }
    acc.iter()
        .map(|(k, v)| format!("{v} {k}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn single_line(s: &str, max: usize) -> String {
    let one = s.replace(['\n', '\r'], " ");
    if one.len() <= max {
        one
    } else {
        let mut end = max.saturating_sub(1);
        while !one.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…", &one[..end])
    }
}

// ============================================================
//  Brief command — `git forum brief` orchestration
// ============================================================

/// Args for [`run`] — `git forum brief`.
pub struct BriefArgs {
    pub thread_id: String,
    pub json: bool,
}

/// Uniform entry point for the `brief` subcommand.
pub fn run(args: BriefArgs, ctx: &Context) -> Result<(), ForumError> {
    let thread_id = resolve_tid(&ctx.git, &args.thread_id)?;
    let state = thread::replay_thread(&ctx.git, &thread_id)?;
    let incoming = read_incoming_link_counts(&ctx.paths, &thread_id);
    if args.json {
        let payload = build_json(&state, &incoming);
        let s =
            serde_json::to_string_pretty(&payload).map_err(|e| ForumError::Repo(e.to_string()))?;
        println!("{s}");
    } else {
        print!("{}", render_plaintext(&state, &incoming));
    }
    Ok(())
}

/// Read incoming-link counts grouped by relation for `brief`.
///
/// Phase 2 slot 11: the SQLite index is on the Phase 4 DELETE list,
/// so this returns the zero-counts default. SPEC-3.0 §9.2: the
/// index is optional acceleration; the reverse-link query stays
/// available as a tree scan if a future slot needs it.
pub fn read_incoming_link_counts(
    _paths: &config::RepoPaths,
    _thread_id: &str,
) -> IncomingLinkCounts {
    IncomingLinkCounts::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::event::{Lifecycle, ThreadKind, ThreadStatus};
    use crate::internal::node::Node;
    use crate::internal::thread::ThreadLink;
    use chrono::{TimeZone, Utc};

    fn make_state() -> ThreadState {
        // Phase 2c: lifecycle is now an independent field (no longer derived
        // from kind on read), so test fixtures must set it explicitly to
        // match the kind-implied value.
        let t = Utc.with_ymd_and_hms(2026, 4, 30, 12, 0, 0).unwrap();
        ThreadState {
            id: "RFC-x9k2".into(),
            kind: ThreadKind::Rfc,
            title: "Replace LIKE scan with FTS5".into(),
            status: ThreadStatus::Done,
            lifecycle: Lifecycle::Proposal,
            created_at: t,
            created_by: "ai/claude".into(),
            tags: vec!["cross-cutting".into()],
            links: vec![ThreadLink {
                target_thread_id: "RFC-other".into(),
                rel: "relates-to".into(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn plaintext_does_not_name_linked_threads() {
        let state = make_state();
        let mut incoming = IncomingLinkCounts::default();
        incoming.by_rel.insert("implements".into(), 2);
        let out = render_plaintext(&state, &incoming);

        // Counts and relations are present.
        assert!(out.contains("in=2 implements"));
        assert!(out.contains("out=1 relates-to"));

        // No linked thread IDs, titles, or statuses appear in output.
        assert!(!out.contains("RFC-other"));
    }

    #[test]
    fn plaintext_minimal_thread_renders() {
        let state = ThreadState {
            id: "TASK-123".into(),
            kind: ThreadKind::Task,
            title: "Lonely task".into(),
            status: ThreadStatus::Open,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            created_by: "human/alice".into(),
            ..Default::default()
        };
        let out = render_plaintext(&state, &IncomingLinkCounts::default());
        assert!(out.contains("TASK-123"));
        assert!(out.contains("Lonely task"));
        // No links → "in=0, out=0"
        assert!(out.contains("links:    in=0, out=0"));
        // No evidence → "evidence: 0"
        assert!(out.contains("evidence: 0"));
        // No latest summary → "-"
        assert!(out.contains("summary:  -"));
    }

    #[test]
    fn json_schema_has_required_fields() {
        let state = make_state();
        let mut incoming = IncomingLinkCounts::default();
        incoming.by_rel.insert("implements".into(), 2);
        let json = build_json(&state, &incoming);

        assert_eq!(json.id, "RFC-x9k2");
        assert_eq!(json.lifecycle, "proposal");
        assert_eq!(json.status, "done");
        assert_eq!(json.tags, vec!["cross-cutting".to_string()]);
        assert_eq!(json.links_in.len(), 1);
        assert_eq!(json.links_in[0].rel, "implements");
        assert_eq!(json.links_in[0].count, 2);
        assert_eq!(json.links_out.len(), 1);
        assert_eq!(json.links_out[0].rel, "relates-to");
    }

    #[test]
    fn node_counts_collapse_legacy_variants() {
        let state = ThreadState {
            id: "RFC-1".into(),
            kind: ThreadKind::Rfc,
            title: "T".into(),
            status: ThreadStatus::Draft,
            created_at: Utc::now(),
            created_by: "ai/x".into(),
            nodes: vec![
                Node {
                    node_type: NodeType::Claim,
                    ..Default::default()
                },
                Node {
                    node_type: NodeType::Question,
                    ..Default::default()
                },
                Node {
                    node_type: NodeType::Approval,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let counts = canonical_node_counts(&state.nodes);
        // Claim + Question collapse to comment; Approval is its own bucket.
        assert_eq!(counts.get("comment"), Some(&2));
        assert_eq!(counts.get("approval"), Some(&1));
    }
}
