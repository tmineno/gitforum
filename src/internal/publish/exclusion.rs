//! Pure exclusion pipeline for the published namespace (RFC
//! `fls856j3` §5.5).
//!
//! Given a public thread's [`ThreadDocument`] and the set of thread
//! IDs known to be public, this module produces the materialised
//! published `ThreadDocument` by:
//!
//! 1. Dropping `links.toml` entries whose `target` is **non-public**
//!    — i.e. not in the public allowlist. This catches private
//!    targets *and* unknown/absent targets (the publisher cannot
//!    verify a target it has no authoritative ref for, so it is
//!    treated as non-public).
//! 2. Dropping `evidence.toml` entries with `kind = "thread"` whose
//!    target is non-public.
//! 3. Passing `body.md`, `nodes/*.md`, and `nodes/*.toml` through
//!    unchanged. The publisher does not rewrite text bytes; the
//!    pre-publish lint module is informational.
//!
//! The transform is a pure function of `(source ThreadDocument,
//! public set)` so identical input partitions produce identical
//! published trees — RFC §5.5's tree-equivalence idempotency property.

use std::collections::HashSet;

use crate::internal::evidence::EvidenceKind;
use crate::internal::snapshot::ThreadDocument;

/// Apply the published-namespace exclusion rules to `doc` in place,
/// using `public_ids` as the allowlist of thread IDs known to be
/// public locally.
///
/// Preconditions: `doc.snapshot.visibility == Visibility::Public`
/// (callers should not invoke this on a private thread; the
/// publisher must have already filtered the thread set). This
/// function does not check that itself — it is a pure tree
/// transform.
/// Postconditions: `doc.links.entries` and `doc.evidence.entries`
/// contain no entries pointing at any id outside `public_ids`
/// (private + unknown + absent are all dropped). All other fields
/// are unchanged byte-for-byte.
/// Failure modes: none — the transform never fails.
/// Side effects: none — `doc` is mutated, no I/O.
pub fn apply(doc: &mut ThreadDocument, public_ids: &HashSet<String>) {
    doc.links
        .entries
        .retain(|link| public_ids.contains(&link.target));

    doc.evidence
        .entries
        .retain(|ev| !(ev.kind == EvidenceKind::Thread) || public_ids.contains(&ev.ref_target));
}

/// Convenience: take a snapshot by value, apply [`apply`], return
/// the filtered tree. Useful when the caller has the source
/// document but does not need the original after publishing.
pub fn filter(mut doc: ThreadDocument, public_ids: &HashSet<String>) -> ThreadDocument {
    apply(&mut doc, public_ids);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;

    use crate::internal::evidence::{EvidenceFile, EvidenceRecord};
    use crate::internal::snapshot::link::{Link, Links};
    use crate::internal::snapshot::store::NodeWithBody;
    use crate::internal::thread::{ThreadSnapshot, Visibility};

    fn epoch() -> chrono::DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn public_doc(id: &str) -> ThreadDocument {
        ThreadDocument {
            snapshot: ThreadSnapshot {
                schema_version: 3,
                id: id.into(),
                title: "Public thread".into(),
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
            body: Some("body bytes\nwith @priv1234 reference\n".into()),
            nodes: vec![],
            links: Links { entries: vec![] },
            evidence: EvidenceFile { entries: vec![] },
        }
    }

    fn link(target: &str, rel: &str) -> Link {
        Link {
            target: target.into(),
            rel: rel.into(),
            created_at: epoch(),
            created_by: "human/alice".into(),
        }
    }

    fn ev_thread(id: &str, target: &str) -> EvidenceRecord {
        EvidenceRecord {
            id: id.into(),
            kind: EvidenceKind::Thread,
            ref_target: target.into(),
            created_at: epoch(),
            created_by: "human/alice".into(),
        }
    }

    fn ev_commit(id: &str, sha: &str) -> EvidenceRecord {
        EvidenceRecord {
            id: id.into(),
            kind: EvidenceKind::Commit,
            ref_target: sha.into(),
            created_at: epoch(),
            created_by: "human/alice".into(),
        }
    }

    fn public_set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn drops_links_outside_the_public_allowlist() {
        let mut doc = public_doc("pub00000");
        doc.links.entries = vec![
            link("pub11111", "depends-on"), // public — kept
            link("priv1234", "blocks"),     // private — dropped
            link("pub22222", "relates-to"), // public — kept
            link("unknown1", "related"),    // unknown — dropped
        ];

        apply(&mut doc, &public_set(&["pub11111", "pub22222"]));

        assert_eq!(doc.links.entries.len(), 2);
        assert_eq!(doc.links.entries[0].target, "pub11111");
        assert_eq!(doc.links.entries[1].target, "pub22222");
    }

    #[test]
    fn drops_thread_evidence_outside_the_public_allowlist() {
        let mut doc = public_doc("pub00000");
        doc.evidence.entries = vec![
            ev_thread("ev01", "pub11111"),
            ev_thread("ev02", "priv1234"), // dropped
            ev_thread("ev03", "unknown1"), // dropped (unknown target)
            ev_commit("ev04", "deadbeefdeadbeef"),
            // Hunk/file evidence carries a SHA or path string in
            // `ref_target`; non-thread kinds are never gated by the
            // public allowlist.
            EvidenceRecord {
                id: "ev05".into(),
                kind: EvidenceKind::File,
                ref_target: "priv1234".into(),
                created_at: epoch(),
                created_by: "human/alice".into(),
            },
        ];

        apply(&mut doc, &public_set(&["pub11111"]));

        let ids: Vec<&str> = doc.evidence.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["ev01", "ev04", "ev05"]);
    }

    #[test]
    fn body_and_nodes_pass_through_unchanged() {
        let mut doc = public_doc("pub00000");
        let original_body = doc.body.clone();
        doc.nodes = vec![
            NodeWithBody {
                body: "node body referencing @priv1234 and priv5678".into(),
                ..Default::default()
            },
            NodeWithBody {
                body: "another node".into(),
                ..Default::default()
            },
        ];
        let original_nodes = doc.nodes.clone();

        apply(&mut doc, &public_set(&[]));

        assert_eq!(doc.body, original_body, "body bytes must be byte-identical");
        assert_eq!(
            doc.nodes, original_nodes,
            "node bodies and metadata must be byte-identical"
        );
    }

    #[test]
    fn empty_public_set_drops_all_thread_targets() {
        // No allowlist → drop every link and every thread evidence,
        // since the publisher cannot prove any target is public.
        let mut doc = public_doc("pub00000");
        doc.links.entries = vec![link("pub11111", "depends-on")];
        doc.evidence.entries = vec![ev_thread("ev01", "pub11111")];

        apply(&mut doc, &HashSet::new());

        assert!(doc.links.entries.is_empty());
        assert!(doc.evidence.entries.is_empty());
    }

    #[test]
    fn deterministic_across_repeated_application() {
        // Tree-equivalence (RFC §5.5): same input → same output.
        let mut a = public_doc("pub00000");
        let mut b = public_doc("pub00000");

        let entries = vec![
            link("priv1234", "blocks"),
            link("pub11111", "depends-on"),
            link("priv5678", "supersedes"),
        ];
        a.links.entries = entries.clone();
        b.links.entries = entries;
        a.evidence.entries = vec![ev_thread("ev01", "priv1234")];
        b.evidence.entries = vec![ev_thread("ev01", "priv1234")];

        let public = public_set(&["pub11111"]);
        apply(&mut a, &public);
        apply(&mut b, &public);

        assert_eq!(a, b);
    }

    #[test]
    fn filter_returns_owned_value() {
        let doc = public_doc("pub00000");
        let original_id = doc.snapshot.id.clone();
        let filtered = filter(doc, &public_set(&[]));
        assert_eq!(filtered.snapshot.id, original_id);
    }
}
