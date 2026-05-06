//! Pre-publish lint per RFC `fls856j3` §4.4.
//!
//! Pure scan of a public thread's body and node text for tokens that
//! name a thread the local index marks as **private**. The lint is
//! informational — it never rewrites content and never blocks the
//! publish by itself. Callers (`git forum push` orchestration) decide
//! whether warnings exit non-zero (`--strict`) or print-and-proceed
//! (default).
//!
//! Forms scanned:
//!
//! - `@<id>` display form.
//! - Full ref form `refs/forum/threads/<id>`.
//! - Labeled-context bare IDs immediately following `Refs:`,
//!   `thread:`, `parent:`, or `reply_to:` markers.
//! - Bare 8-char base36 tokens that **exact-match** a known private
//!   thread ID. The exact-match constraint avoids the false-positive
//!   problem that killed the bare-ID scrubber: an abbreviated commit
//!   hash or base36 nonce that does not equal a known private ID
//!   does not warn.
//!
//! All forms produce warnings only when the matched 8-char token is
//! in `private_ids` — see `LintForm` for the per-form classification
//! used in the warning message.

use std::collections::HashSet;

use crate::internal::snapshot::ThreadDocument;

/// One pre-publish lint hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintWarning {
    pub thread_id: String,
    /// `None` when the match is in `body.md`; `Some(node_id)` when the
    /// match is inside a node body.
    pub node_id: Option<String>,
    pub form: LintForm,
    pub matched_id: String,
}

impl LintWarning {
    /// Compact one-line representation per RFC §4.4: `<thread>:<node>:<form>:<id>`.
    pub fn render(&self) -> String {
        let node = self.node_id.as_deref().unwrap_or("body");
        let form = self.form.label();
        format!("{}:{}:{}:{}", self.thread_id, node, form, self.matched_id)
    }
}

/// Surface form a private-id reference was matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintForm {
    /// `@<id>` display form.
    AtId,
    /// `refs/forum/threads/<id>` full ref form.
    FullRef,
    /// Bare ID immediately following `Refs:`, `thread:`, `parent:`, or
    /// `reply_to:` markers (possibly through a comma-separated list).
    Labeled,
    /// Bare 8-char token matching a known-private ID by exact lookup.
    BareToken,
}

impl LintForm {
    pub fn label(&self) -> &'static str {
        match self {
            LintForm::AtId => "at-id",
            LintForm::FullRef => "full-ref",
            LintForm::Labeled => "labeled",
            LintForm::BareToken => "bare-token",
        }
    }
}

/// Scan a public thread's body and nodes, returning one warning per
/// match.
///
/// Preconditions: caller has already restricted attention to threads
/// being materialised into the published namespace (i.e. `doc.snapshot.visibility
/// == Visibility::Public`); this function does not check that.
/// Postconditions: returns a vector of [`LintWarning`] in
/// document-source order — body matches first, then nodes in
/// `doc.nodes` order.
/// Failure modes: none — scanning never errors.
/// Side effects: none — read-only over `doc`.
pub fn scan(doc: &ThreadDocument, private_ids: &HashSet<String>) -> Vec<LintWarning> {
    if private_ids.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(body) = doc.body.as_deref() {
        scan_text(body, &doc.snapshot.id, None, private_ids, &mut out);
    }
    for node in &doc.nodes {
        scan_text(
            &node.body,
            &doc.snapshot.id,
            Some(&node.record.id),
            private_ids,
            &mut out,
        );
    }
    out
}

/// Thread IDs are SPEC-3.0 §6 8-character base36 tokens.
const ID_LEN: usize = 8;

fn is_id_char(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit()
}

/// Scan one text blob (body or node body) for matches. The scanner
/// walks the text byte-by-byte, finding every 8-char base36 token
/// with non-id-char boundaries on both sides, then classifies the
/// surrounding context.
///
/// Tokens are reported only when they are present in `private_ids`
/// — this is the exact-match property that distinguishes the lint
/// from the rejected bare-id scrubber (RFC §4.4).
fn scan_text(
    text: &str,
    thread_id: &str,
    node_id: Option<&str>,
    private_ids: &HashSet<String>,
    out: &mut Vec<LintWarning>,
) {
    let bytes = text.as_bytes();
    let n = bytes.len();
    if n < ID_LEN {
        return;
    }

    let mut i = 0;
    while i + ID_LEN <= n {
        // Token boundary check: byte before must not be id-char.
        if i > 0 && is_id_char(bytes[i - 1]) {
            i += 1;
            continue;
        }
        // The 8 candidate bytes must all be id-char.
        let mut all_id = true;
        for &b in &bytes[i..i + ID_LEN] {
            if !is_id_char(b) {
                all_id = false;
                break;
            }
        }
        if !all_id {
            // Skip past the non-id char so we don't re-examine it.
            i += 1;
            continue;
        }
        // Trailing boundary check.
        if i + ID_LEN < n && is_id_char(bytes[i + ID_LEN]) {
            // Inside a longer alphanumeric run — skip the whole run.
            let mut j = i + ID_LEN;
            while j < n && is_id_char(bytes[j]) {
                j += 1;
            }
            i = j;
            continue;
        }

        // Candidate token at [i, i+ID_LEN).
        let token = &text[i..i + ID_LEN];
        if private_ids.contains(token) {
            let form = classify_form(text, i);
            out.push(LintWarning {
                thread_id: thread_id.to_string(),
                node_id: node_id.map(|s| s.to_string()),
                form,
                matched_id: token.to_string(),
            });
        }
        i += ID_LEN;
    }
}

/// Inspect the bytes immediately before `start` (the position of a
/// matched 8-char token) and decide which surface form classifies
/// this match.
fn classify_form(text: &str, start: usize) -> LintForm {
    let prefix = &text[..start];

    if prefix.ends_with('@') {
        return LintForm::AtId;
    }
    if prefix.ends_with("refs/forum/threads/") {
        return LintForm::FullRef;
    }

    // Labeled-context: walk back over whitespace, commas, and
    // id-char runs (each entry of a comma list is itself a base36
    // token). When we run out of those, check for a label marker.
    // This catches `Refs: priv1234, abc12345, priv5678` for every
    // entry in the list, not just the first.
    let bytes = prefix.as_bytes();
    let mut j = bytes.len();
    while j > 0 {
        let c = bytes[j - 1];
        if c == b' ' || c == b'\t' || c == b',' || is_id_char(c) {
            j -= 1;
            continue;
        }
        break;
    }
    let trimmed = &prefix[..j];
    if has_label_marker(trimmed) {
        return LintForm::Labeled;
    }

    LintForm::BareToken
}

/// Does `prefix` end with one of the labeled-context markers
/// (`Refs:`, `thread:`, `parent:`, `reply_to:`)?
///
/// The check is whitespace-tolerant on the inside (`Refs : `) but
/// requires the marker to be at the *end* of `prefix` after the
/// caller has already trimmed trailing whitespace and commas
/// (we want to match `... Refs: id` AND `... Refs: a, id`).
fn has_label_marker(prefix: &str) -> bool {
    // Marker syntax: the keyword followed by optional whitespace
    // and a `:`. We find the trailing `:` then check the keyword
    // before it.
    let bytes = prefix.as_bytes();
    let mut k = bytes.len();
    while k > 0 && (bytes[k - 1] == b' ' || bytes[k - 1] == b'\t') {
        k -= 1;
    }
    if k == 0 || bytes[k - 1] != b':' {
        return false;
    }
    let before_colon = &prefix[..k - 1];
    let trimmed = before_colon.trim_end();
    for marker in ["Refs", "thread", "parent", "reply_to"] {
        if trimmed.ends_with(marker) {
            // Boundary: the char before `marker` (if any) must not
            // be id-char-like — otherwise something like
            // `notRefs:` would match.
            let head_len = trimmed.len() - marker.len();
            if head_len == 0 {
                return true;
            }
            let prev = trimmed.as_bytes()[head_len - 1];
            if !prev.is_ascii_alphanumeric() && prev != b'_' {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;

    use crate::internal::evidence::EvidenceFile;
    use crate::internal::node::{NodeKind, NodeRecord, NodeStatus};
    use crate::internal::snapshot::link::Links;
    use crate::internal::snapshot::store::NodeWithBody;
    use crate::internal::thread::{ThreadSnapshot, Visibility};

    fn epoch() -> chrono::DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn doc_with_body(body: &str) -> ThreadDocument {
        ThreadDocument {
            snapshot: ThreadSnapshot {
                schema_version: 3,
                id: "pub00000".into(),
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
            body: Some(body.into()),
            nodes: vec![],
            links: Links { entries: vec![] },
            evidence: EvidenceFile { entries: vec![] },
        }
    }

    fn private(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn empty_private_set_yields_no_warnings() {
        let doc = doc_with_body("see @priv1234 and refs/forum/threads/priv5678");
        let warnings = scan(&doc, &HashSet::new());
        assert!(warnings.is_empty());
    }

    #[test]
    fn at_id_form_is_caught() {
        let doc = doc_with_body("blocked by @priv1234 right now");
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].form, LintForm::AtId);
        assert_eq!(warnings[0].matched_id, "priv1234");
        assert!(warnings[0].node_id.is_none());
    }

    #[test]
    fn full_ref_form_is_caught() {
        let doc = doc_with_body("see refs/forum/threads/priv1234 for details");
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].form, LintForm::FullRef);
    }

    #[test]
    fn labeled_refs_marker_caught() {
        let body = "summary line\n\nRefs: priv1234\n";
        let doc = doc_with_body(body);
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].form, LintForm::Labeled);
    }

    #[test]
    fn labeled_refs_with_list_catches_each_entry() {
        let body = "Refs: priv1234, abc12345, priv5678\n";
        let doc = doc_with_body(body);
        let warnings = scan(&doc, &private(&["priv1234", "priv5678"]));
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().all(|w| w.form == LintForm::Labeled));
        let ids: Vec<&str> = warnings.iter().map(|w| w.matched_id.as_str()).collect();
        assert_eq!(ids, vec!["priv1234", "priv5678"]);
    }

    #[test]
    fn bare_token_form_used_when_no_marker() {
        let body = "see priv1234 in the next paragraph";
        let doc = doc_with_body(body);
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].form, LintForm::BareToken);
    }

    #[test]
    fn unknown_bare_tokens_do_not_warn() {
        // Abbreviated commit hash, base36 nonce — neither is in the
        // private set, so the lint stays silent.
        let body = "fixed in deadbeef and unrelated 9z8a7b6c";
        let doc = doc_with_body(body);
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert!(warnings.is_empty());
    }

    #[test]
    fn longer_runs_skipped() {
        // 9-char alphanumeric token must not be matched as an
        // 8-char window. Boundary-aware scanning prevents this.
        let body = "longer token: priv12345abc";
        let doc = doc_with_body(body);
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert!(warnings.is_empty(), "got: {warnings:?}");
    }

    #[test]
    fn nodes_are_scanned_too() {
        let mut doc = doc_with_body("body has no private ids");
        doc.nodes.push(NodeWithBody {
            record: NodeRecord {
                id: "node0001".into(),
                kind: NodeKind::Comment,
                status: NodeStatus::Open,
                created_at: epoch(),
                created_by: "human/alice".into(),
                ..Default::default()
            },
            body: "this node references @priv1234".into(),
        });
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].node_id.as_deref(), Some("node0001"));
        assert_eq!(warnings[0].form, LintForm::AtId);
    }

    #[test]
    fn render_format_matches_rfc_one_liner() {
        let w = LintWarning {
            thread_id: "pub00000".into(),
            node_id: Some("node0001".into()),
            form: LintForm::AtId,
            matched_id: "priv1234".into(),
        };
        assert_eq!(w.render(), "pub00000:node0001:at-id:priv1234");
        let body_w = LintWarning { node_id: None, ..w };
        assert_eq!(body_w.render(), "pub00000:body:at-id:priv1234");
    }

    #[test]
    fn at_id_takes_priority_over_bare_token() {
        // `@priv1234` matches the AtId form, not BareToken.
        let body = "@priv1234 is the form";
        let doc = doc_with_body(body);
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].form, LintForm::AtId);
    }

    #[test]
    fn full_ref_does_not_double_match_as_bare() {
        // Inside `refs/forum/threads/priv1234` the 8 chars right
        // after the slash form the token. The classifier inspects
        // the prefix so the form is FullRef. Verify only one warning
        // per token.
        let body = "context: refs/forum/threads/priv1234 end.";
        let doc = doc_with_body(body);
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].form, LintForm::FullRef);
    }

    #[test]
    fn determinism_same_input_same_output() {
        let body = "@priv1234 and Refs: priv1234, priv5678. Also see priv1234 again.";
        let doc = doc_with_body(body);
        let p = private(&["priv1234", "priv5678"]);
        let a = scan(&doc, &p);
        let b = scan(&doc, &p);
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn underscored_marker_does_not_match_label() {
        // `notRefs:` should NOT count as `Refs:`.
        let body = "notRefs: priv1234";
        let doc = doc_with_body(body);
        let warnings = scan(&doc, &private(&["priv1234"]));
        assert_eq!(warnings.len(), 1);
        // The id is still bare-tokened — exact-match still warns,
        // just under the BareToken classification.
        assert_eq!(warnings[0].form, LintForm::BareToken);
    }
}
