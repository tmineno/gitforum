//! SPEC-3.0 §3.1 / §3.2 / §3.3 policy and category registry.
//!
//! 3.0 policy is a flat per-category bundle:
//!
//! ```toml
//! [categories.rfc.guards]
//! "review->done" = ["one_approval", "no_open_objections"]
//!
//! [categories.task.creation]
//! required_body = false
//! body_sections = ["Background", "Acceptance criteria"]
//! ```
//!
//! There is no scope-expression DSL (SPEC-3.0 §1.1: "no string expression
//! parser"). 3.0 has no tag/lifecycle selector language (§3.1: "3.0 does
//! not define a selector language over tags or other facets"). Legacy 2.x
//! shapes (`[[guards]] on = "..." requires = [...]`, kind/lifecycle/
//! facet-scoped `creation_rules.*`, `node_rules`, `revise_rules`,
//! `evidence_rules`, the `one_human_approval` and `at_least_one_summary`
//! rule names) are detected at load time and rejected with a hint
//! pointing at `git forum migrate --to 3.0`.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::error::{ForumError, ForumResult};
use super::evidence::EvidenceKind;
use super::lint_emit::{self, LintEmitter};
use super::node::{NodeKind, NodeType};
use super::thread::ThreadState;

// --------------------------------------------------------------------
// `Lifecycle` (3-variant v2 enum) was relocated here from `event.rs`
// in Phase 4 Step 1j (RFC `7ymtc4b2`, task `913c4s9v`). Co-located
// with the existing v2 ↔ 3.0 mapping helpers
// (`lifecycle_to_category`, `legacy_lifecycle_for_category`,
// `category_for_state`) so the entire v2 state-machine surface lives
// in one place.
//
// SPEC-3.0 §3 replaces Lifecycle dispatch with the
// `CategoryRegistry` (defined further down). v3.0.0 read paths still
// use Lifecycle for legacy snapshots; the full Lifecycle removal is
// a v3.1 follow-up — v3.0.0 just relocates the enum to a 3.0-native
// home so KEEP files don't reach into `internal::event`.
// --------------------------------------------------------------------

/// SPEC-2.0 §2.3.1 — the sole required facet, gates the unified state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    Proposal,
    #[default]
    Execution,
    Record,
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposal => "proposal",
            Self::Execution => "execution",
            Self::Record => "record",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "proposal" => Some(Self::Proposal),
            "execution" => Some(Self::Execution),
            "record" => Some(Self::Record),
            _ => None,
        }
    }

    /// SPEC-2.0 §3.1.1 — initial state per lifecycle.
    pub fn initial_state(self) -> &'static str {
        super::workflow::SPEC.initial_state(self)
    }

    /// SPEC-2.0 §3.1.1 — states reachable for this lifecycle.
    pub fn allowed_states(self) -> &'static [&'static str] {
        super::workflow::SPEC.allowed_states(self)
    }

    pub fn allows_state(self, state: &str) -> bool {
        super::workflow::SPEC.allows_state(self, state)
    }
}

impl std::fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------
// SPEC-3.0 §3.1 category registry
// ---------------------------------------------------------------------

/// One category's status machine per SPEC-3.0 §3.1.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CategoryDefinition {
    pub initial_status: String,
    pub statuses: Vec<String>,
    /// `from->to` strings, e.g. `"draft->open"`.
    pub transitions: Vec<String>,
}

impl CategoryDefinition {
    pub fn has_status(&self, status: &str) -> bool {
        self.statuses.iter().any(|s| s == status)
    }

    pub fn allows_transition(&self, from: &str, to: &str) -> bool {
        let needle = format!("{from}->{to}");
        self.transitions.iter().any(|t| t == &needle)
    }

    /// Destination statuses reachable in one step from `from`.
    pub fn valid_targets(&self, from: &str) -> Vec<String> {
        let prefix = format!("{from}->");
        self.transitions
            .iter()
            .filter_map(|t| t.strip_prefix(&prefix).map(String::from))
            .collect()
    }

    /// BFS shortest path from `from` to `to` using this category's
    /// `transitions`. Returns the sequence of intermediate destinations
    /// (excluding `from`, including `to`), or `None` when unreachable.
    pub fn find_path(&self, from: &str, to: &str) -> Option<Vec<String>> {
        if from == to {
            return Some(Vec::new());
        }
        let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
        for edge in &self.transitions {
            if let Some((f, t)) = edge.split_once("->") {
                adjacency.entry(f.trim()).or_default().push(t.trim());
            }
        }
        let mut visited: HashMap<String, Option<String>> = HashMap::new();
        let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        visited.insert(from.to_string(), None);
        queue.push_back(from.to_string());
        while let Some(node) = queue.pop_front() {
            if node == to {
                let mut path = Vec::new();
                let mut cursor = node;
                while let Some(prev) = visited.get(&cursor).cloned().flatten() {
                    path.push(cursor);
                    cursor = prev;
                }
                path.reverse();
                return Some(path);
            }
            if let Some(neighbors) = adjacency.get(node.as_str()) {
                for &n in neighbors {
                    if !visited.contains_key(n) {
                        visited.insert(n.to_string(), Some(node.clone()));
                        queue.push_back(n.to_string());
                    }
                }
            }
        }
        None
    }
}

/// Repository-level category map per SPEC-3.0 §3.1.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CategoryRegistry {
    pub categories: HashMap<String, CategoryDefinition>,
}

impl CategoryRegistry {
    /// Built-in categories that every native 3.0 implementation MUST
    /// always provide (SPEC-3.0 §3.1).
    pub fn built_in() -> Self {
        let mut categories = HashMap::new();
        categories.insert("rfc".into(), built_in_rfc());
        categories.insert("task".into(), built_in_task());
        Self { categories }
    }

    pub fn get(&self, category: &str) -> Option<&CategoryDefinition> {
        self.categories.get(category)
    }

    /// Validate that `status` is an allowed status for `category`.
    /// Returns `Err` for unknown category or unknown status.
    pub fn validate_status(&self, category: &str, status: &str) -> Result<(), ForumError> {
        let def = self
            .get(category)
            .ok_or_else(|| ForumError::SnapshotInvalid(format!("unknown category `{category}`")))?;
        if !def.has_status(status) {
            return Err(ForumError::SnapshotInvalid(format!(
                "category `{category}` does not allow status `{status}`"
            )));
        }
        Ok(())
    }
}

fn built_in_rfc() -> CategoryDefinition {
    // The SPEC-3.0 §3.1 example registry table is the structural
    // baseline. Two reopen edges (`done->open`, `rejected->open`) are
    // added beyond the example so the everyday `git forum reopen <ID>`
    // shorthand keeps working without per-repo overrides.
    CategoryDefinition {
        initial_status: "draft".into(),
        statuses: [
            "draft",
            "open",
            "review",
            "done",
            "rejected",
            "withdrawn",
            "deprecated",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        transitions: [
            "draft->open",
            "draft->withdrawn",
            "open->review",
            "open->rejected",
            "open->withdrawn",
            "review->done",
            "review->rejected",
            "done->open",
            "rejected->open",
            "done->deprecated",
            "rejected->deprecated",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    }
}

fn built_in_task() -> CategoryDefinition {
    CategoryDefinition {
        initial_status: "open".into(),
        statuses: [
            "open",
            "working",
            "review",
            "done",
            "rejected",
            "deprecated",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        transitions: [
            "open->working",
            "open->review",
            "open->done",
            "open->rejected",
            "working->review",
            "working->done",
            "working->rejected",
            "review->done",
            "review->working",
            "review->rejected",
            "done->open",
            "rejected->open",
            "done->deprecated",
            "rejected->deprecated",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    }
}

// ---------------------------------------------------------------------
// SPEC-3.0 §8.3 lifecycle ↔ category mapping
// ---------------------------------------------------------------------

/// SPEC-3.0 §8.3: legacy lifecycle → 3.0 category mapping. Used by both
/// the read-side adapter (`category_for_state`) and the migration
/// projection.
pub fn lifecycle_to_category(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Proposal => "rfc",
        Lifecycle::Execution | Lifecycle::Record => "task",
    }
}

/// Inverse helper for the few read paths that still need a `Lifecycle`
/// value. Per SPEC-3.0 §8.3 the `task` category covers both Execution
/// and Record; the `decision` tag distinguishes the two on read.
pub fn legacy_lifecycle_for_category(category: &str, tags: &[String]) -> Lifecycle {
    match category {
        "task" => {
            if tags.iter().any(|t| t == "decision") {
                Lifecycle::Record
            } else {
                Lifecycle::Execution
            }
        }
        _ => Lifecycle::Proposal,
    }
}

/// Resolve a thread state's 3.0 category from its lifecycle facet.
///
/// Used by the v2 read path (`ThreadState`-bearing callers) to pick the
/// right `[categories.<NAME>]` slice in the 3.0 policy.
pub fn category_for_state(state: &ThreadState) -> &'static str {
    lifecycle_to_category(state.lifecycle)
}

/// SPEC-2.0 §3.1.2 — pure text-level normalization of 1.x state names
/// to 2.0. Phase 4 Step 1i (RFC `7ymtc4b2`, task `913c4s9v`) relocated
/// this from `event.rs`; `internal::event::normalize_state_name`
/// remains as a `pub use` re-export for legacy / DELETE-list callers.
///
/// Thin wrapper that re-exports [`super::legacy::v1::normalize_state_name`].
/// New domain code should call into [`super::legacy::v1`] directly per
/// RFC 915yuegd P1.
pub fn normalize_state_name(s: &str) -> &str {
    super::legacy::v1::normalize_state_name(s)
}

// ---------------------------------------------------------------------
// SPEC-3.0 §3.2 guard rules
// ---------------------------------------------------------------------

/// 3.0 guard rule names per SPEC-3.0 §3.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardRule {
    /// The thread has no `objection` node with `status = "open"`.
    NoOpenObjections,
    /// The thread has no `action` node with `status = "open"`.
    NoOpenActions,
    /// At least one non-retracted `approval` node exists, regardless of
    /// actor type. (SPEC-3.0 §3.2: replaces v2 `one_human_approval`.)
    OneApproval,
    /// The thread has at least one evidence entry with `kind = "commit"`.
    HasCommitEvidence,
}

impl std::fmt::Display for GuardRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOpenObjections => write!(f, "no_open_objections"),
            Self::NoOpenActions => write!(f, "no_open_actions"),
            Self::OneApproval => write!(f, "one_approval"),
            Self::HasCommitEvidence => write!(f, "has_commit_evidence"),
        }
    }
}

// ---------------------------------------------------------------------
// SPEC-3.0 §3.3 per-category rule bundles
// ---------------------------------------------------------------------

/// Body / sections required at thread creation (SPEC-3.0 §3.3).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CreationRules {
    #[serde(default)]
    pub required_body: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_sections: Vec<String>,
}

/// Statuses in which body / node revision is allowed.
///
/// SPEC-3.0 §3.3: an absent allow list (`None`) means no restriction;
/// a present-but-empty list (`Some(vec![])`) explicitly denies the
/// operation in every status.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ReviseRules {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_body_revise: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_node_revise: Option<Vec<String>>,
}

/// Statuses in which evidence may be attached. Same absent-vs-empty
/// distinction as [`ReviseRules`].
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EvidenceRules {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_evidence: Option<Vec<String>>,
}

/// Global checks (top-level, not category-scoped).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ChecksConfig {
    #[serde(default)]
    pub strict: bool,
}

/// Per-category policy bundle (SPEC-3.0 §3.1 / §3.3).
///
/// `initial_status`, `statuses`, and `transitions` are the §3.1
/// category-definition fields. When all three are present, the entry
/// overrides the built-in for this category (or defines a new
/// non-built-in category). When all three are absent, the built-in
/// definition (if any) is used. Partial overrides are rejected by
/// [`validate_against_registry`].
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CategoryPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statuses: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transitions: Option<Vec<String>>,

    /// Guard rules keyed by `"FROM->TO"` transition string.
    /// Per SPEC-3.0 §3.2: `[categories.rfc.guards] "review->done" = [...]`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub guards: HashMap<String, Vec<GuardRule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation: Option<CreationRules>,
    /// Allowed node types per status. Empty list = no node types allowed
    /// in that status.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub allowed_node_types: HashMap<String, Vec<NodeKind>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revise: Option<ReviseRules>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceRules>,
}

impl CategoryPolicy {
    /// Whether all three §3.1 definition fields are present.
    fn has_full_definition(&self) -> bool {
        self.initial_status.is_some() && self.statuses.is_some() && self.transitions.is_some()
    }

    /// Whether any of the three §3.1 definition fields are present.
    fn has_any_definition_field(&self) -> bool {
        self.initial_status.is_some() || self.statuses.is_some() || self.transitions.is_some()
    }

    /// Project the §3.1 definition fields into a [`CategoryDefinition`].
    /// Caller must ensure [`Self::has_full_definition`] first.
    fn to_definition(&self) -> CategoryDefinition {
        CategoryDefinition {
            initial_status: self.initial_status.clone().unwrap(),
            statuses: self.statuses.clone().unwrap(),
            transitions: self.transitions.clone().unwrap(),
        }
    }
}

/// Parsed policy loaded from `.forum/policy.toml` (SPEC-3.0 §3.1 / §3.2 / §3.3).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Policy {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub categories: HashMap<String, CategoryPolicy>,
    #[serde(default)]
    pub checks: ChecksConfig,
}

impl Policy {
    /// Load and parse policy from the given path. Legacy 2.x shapes
    /// (`[[guards]]`, `requires =`, kind/lifecycle/facet-scoped
    /// `creation_rules.*`, `node_rules`, `revise_rules`, `evidence_rules`,
    /// the `one_human_approval` and `at_least_one_summary` rule names)
    /// are rejected with a hint pointing at `git forum migrate --to 3.0`.
    pub fn load(path: &Path) -> ForumResult<Self> {
        Self::load_with_emitter(path, lint_emit::current())
    }

    /// Like [`Policy::load`], but accepts an explicit lint emitter. The
    /// 3.0 parser does not emit lint warnings during load (legacy shapes
    /// are hard errors, not warnings); the parameter is preserved for
    /// API compatibility with the v2 surface.
    pub fn load_with_emitter(path: &Path, _emitter: &LintEmitter) -> ForumResult<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ForumError::Config(format!("cannot read policy.toml: {e}")))?;

        if let Some(hint) = detect_legacy_policy_form(&text) {
            return Err(ForumError::Config(format!(
                "{hint}\nFile: {}\nRun `git forum migrate --to 3.0` to rewrite legacy \
                 policy.toml to the SPEC-3.0 §3.2/§3.3 category-table form.",
                path.display(),
            )));
        }

        let policy: Self = toml::from_str(&text)
            .map_err(|e| ForumError::Config(format!("invalid policy.toml: {e}")))?;
        validate_against_registry(&policy)?;
        Ok(policy)
    }

    pub fn category(&self, name: &str) -> Option<&CategoryPolicy> {
        self.categories.get(name)
    }

    /// Effective category registry per SPEC-3.0 §3.1: built-in categories
    /// (`rfc`, `task`) overlaid with any per-category definition fields
    /// (`initial_status`, `statuses`, `transitions`) supplied in
    /// `.forum/policy.toml`. Categories that are listed in the policy
    /// without definition fields keep their built-in definition;
    /// non-built-in categories MUST supply all three fields (enforced by
    /// `validate_against_registry`).
    pub fn effective_registry(&self) -> CategoryRegistry {
        let mut registry = CategoryRegistry::built_in();
        for (name, cat) in &self.categories {
            if cat.has_full_definition() {
                registry
                    .categories
                    .insert(name.clone(), cat.to_definition());
            }
        }
        registry
    }

    /// Return the guard rules for `category`'s `from->to` transition, or
    /// `None` when no rules are configured.
    pub fn guards_for_transition(&self, category: &str, transition: &str) -> Option<&[GuardRule]> {
        self.categories
            .get(category)
            .and_then(|c| c.guards.get(transition))
            .map(|v| v.as_slice())
    }

    pub fn creation_rules_for(&self, category: &str) -> Option<&CreationRules> {
        self.categories
            .get(category)
            .and_then(|c| c.creation.as_ref())
    }

    pub fn allowed_node_types(&self, category: &str, status: &str) -> Option<&[NodeKind]> {
        self.categories
            .get(category)
            .and_then(|c| c.allowed_node_types.get(status))
            .map(|v| v.as_slice())
    }

    pub fn revise_rules_for(&self, category: &str) -> Option<&ReviseRules> {
        self.categories
            .get(category)
            .and_then(|c| c.revise.as_ref())
    }

    pub fn evidence_rules_for(&self, category: &str) -> Option<&EvidenceRules> {
        self.categories
            .get(category)
            .and_then(|c| c.evidence.as_ref())
    }
}

// ---------------------------------------------------------------------
// Legacy v2 form detection
// ---------------------------------------------------------------------

/// Scan `text` for any v2/1.x policy shape that 3.0 removes. Returns
/// `Some(hint)` for the first occurrence found, suitable for use as the
/// body of a `ForumError::Config` rejecting the load.
pub fn detect_legacy_policy_form(text: &str) -> Option<String> {
    for (idx, line) in text.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == "[[guards]]" {
            return Some(format!(
                "policy.toml line {lineno}: legacy `[[guards]]` array of tables is removed. \
                 SPEC-3.0 §3.2 uses `[categories.<NAME>.guards]` keyed by transition string"
            ));
        }
        if let Some(rest) = trimmed.strip_prefix("requires") {
            if rest.trim_start().starts_with('=') {
                return Some(format!(
                    "policy.toml line {lineno}: legacy `requires =` field is removed. \
                     SPEC-3.0 §3.2 lists rules as the table-entry value: \
                     `[categories.<NAME>.guards] \"from->to\" = [\"rule\", ...]`"
                ));
            }
        }
        if let Some(rest) = trimmed.strip_prefix('[') {
            let table_name = rest.trim_end_matches(']').trim();
            if table_name.starts_with("creation_rules")
                || table_name.starts_with("node_rules")
                || table_name.starts_with("revise_rules")
                || table_name.starts_with("evidence_rules")
            {
                return Some(format!(
                    "policy.toml line {lineno}: legacy table `[{table_name}]` is removed. \
                     SPEC-3.0 §3.3 uses `[categories.<NAME>.creation]`, `[...allowed_node_types]`, \
                     `[...revise]`, `[...evidence]`"
                ));
            }
        }
        if trimmed.starts_with("on")
            && trimmed.contains('=')
            && (trimmed.contains("lifecycle=")
                || trimmed.contains("kind=")
                || trimmed.contains("tag=")
                || trimmed.contains("AND")
                || trimmed.contains("OR")
                || trimmed.contains("NOT"))
        {
            return Some(format!(
                "policy.toml line {lineno}: legacy facet-scoped guard `on = \"...\"` is removed. \
                 SPEC-3.0 §3.1 has no tag/lifecycle selector language"
            ));
        }
        if line.contains("one_human_approval") {
            return Some(format!(
                "policy.toml line {lineno}: rule `one_human_approval` is removed. \
                 SPEC-3.0 §3.2 uses `one_approval` (counts any non-retracted approval, \
                 regardless of actor type)"
            ));
        }
        if line.contains("at_least_one_summary") {
            return Some(format!(
                "policy.toml line {lineno}: rule `at_least_one_summary` is removed \
                 (SPEC-3.0 §3.2: not a 3.0 rule because `summary` is not a native node type)"
            ));
        }
    }
    None
}

fn validate_against_registry(policy: &Policy) -> ForumResult<()> {
    let built_in = CategoryRegistry::built_in();

    // 1) Validate per-category definition fields, then build the
    //    effective registry. Partial overrides and inconsistent
    //    definitions are rejected here.
    for (cat_name, cat_policy) in &policy.categories {
        let is_built_in = built_in.get(cat_name).is_some();
        if cat_policy.has_any_definition_field() && !cat_policy.has_full_definition() {
            return Err(ForumError::Config(format!(
                "[categories.{cat_name}]: partial category definition; \
                 SPEC-3.0 §3.1 requires all three of `initial_status`, `statuses`, \
                 `transitions` together"
            )));
        }
        if !is_built_in && !cat_policy.has_full_definition() {
            return Err(ForumError::Config(format!(
                "[categories.{cat_name}]: non-built-in category requires `initial_status`, \
                 `statuses`, and `transitions` (SPEC-3.0 §3.1); \
                 built-in categories are: rfc, task"
            )));
        }
        if cat_policy.has_full_definition() {
            validate_definition_consistency(cat_name, cat_policy)?;
        }
    }

    let effective = policy.effective_registry();

    // 2) Validate guards, allowed_node_types, revise, evidence against
    //    the effective definition (built-in or override).
    for (cat_name, cat_policy) in &policy.categories {
        let cat_def = effective
            .get(cat_name)
            .expect("effective registry built above must contain every policy category");

        for transition in cat_policy.guards.keys() {
            let Some((from, to)) = transition.split_once("->") else {
                return Err(ForumError::Config(format!(
                    "[categories.{cat_name}.guards] {transition:?}: invalid transition syntax; \
                     expected \"from->to\""
                )));
            };
            let (from, to) = (from.trim(), to.trim());
            if !cat_def.has_status(from) {
                return Err(ForumError::Config(format!(
                    "[categories.{cat_name}.guards] {transition:?}: status {from:?} is not in \
                     category `{cat_name}`'s `statuses`"
                )));
            }
            if !cat_def.has_status(to) {
                return Err(ForumError::Config(format!(
                    "[categories.{cat_name}.guards] {transition:?}: status {to:?} is not in \
                     category `{cat_name}`'s `statuses`"
                )));
            }
            if !cat_def.allows_transition(from, to) {
                return Err(ForumError::Config(format!(
                    "[categories.{cat_name}.guards] {transition:?}: not a valid transition for \
                     category `{cat_name}`"
                )));
            }
        }

        for status in cat_policy.allowed_node_types.keys() {
            if !cat_def.has_status(status) {
                return Err(ForumError::Config(format!(
                    "[categories.{cat_name}.allowed_node_types] status {status:?} is not in \
                     category `{cat_name}`'s `statuses`"
                )));
            }
        }

        if let Some(revise) = &cat_policy.revise {
            if let Some(list) = &revise.allow_body_revise {
                for s in list {
                    if !cat_def.has_status(s) {
                        return Err(ForumError::Config(format!(
                            "[categories.{cat_name}.revise] allow_body_revise: status {s:?} is \
                             not in category `{cat_name}`'s `statuses`"
                        )));
                    }
                }
            }
            if let Some(list) = &revise.allow_node_revise {
                for s in list {
                    if !cat_def.has_status(s) {
                        return Err(ForumError::Config(format!(
                            "[categories.{cat_name}.revise] allow_node_revise: status {s:?} is \
                             not in category `{cat_name}`'s `statuses`"
                        )));
                    }
                }
            }
        }

        if let Some(evidence) = &cat_policy.evidence {
            if let Some(list) = &evidence.allow_evidence {
                for s in list {
                    if !cat_def.has_status(s) {
                        return Err(ForumError::Config(format!(
                            "[categories.{cat_name}.evidence] allow_evidence: status {s:?} is \
                             not in category `{cat_name}`'s `statuses`"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// SPEC-3.0 §3.1: a category definition with an unknown status, duplicate
/// transition, or transition referencing a status outside `statuses` is
/// invalid. Also: `initial_status` must appear in `statuses`.
fn validate_definition_consistency(cat_name: &str, cat: &CategoryPolicy) -> ForumResult<()> {
    let statuses = cat.statuses.as_ref().expect("checked by caller");
    let transitions = cat.transitions.as_ref().expect("checked by caller");
    let initial_status = cat.initial_status.as_ref().expect("checked by caller");

    if statuses.is_empty() {
        return Err(ForumError::Config(format!(
            "[categories.{cat_name}]: `statuses` must not be empty"
        )));
    }
    if !statuses.iter().any(|s| s == initial_status) {
        return Err(ForumError::Config(format!(
            "[categories.{cat_name}]: `initial_status = {initial_status:?}` is not in \
             `statuses`"
        )));
    }
    let mut seen: std::collections::HashSet<&String> = std::collections::HashSet::new();
    for t in transitions {
        if !seen.insert(t) {
            return Err(ForumError::Config(format!(
                "[categories.{cat_name}]: duplicate transition {t:?}"
            )));
        }
        let Some((from, to)) = t.split_once("->") else {
            return Err(ForumError::Config(format!(
                "[categories.{cat_name}]: transition {t:?}: invalid syntax; expected \
                 \"from->to\""
            )));
        };
        let (from, to) = (from.trim(), to.trim());
        if !statuses.iter().any(|s| s == from) {
            return Err(ForumError::Config(format!(
                "[categories.{cat_name}]: transition {t:?}: status {from:?} is not in \
                 `statuses`"
            )));
        }
        if !statuses.iter().any(|s| s == to) {
            return Err(ForumError::Config(format!(
                "[categories.{cat_name}]: transition {t:?}: status {to:?} is not in \
                 `statuses`"
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Guard violations
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GuardViolation {
    pub rule: String,
    pub reason: String,
}

/// Evaluate guards for `category`'s `from->to` transition against `state`.
///
/// `state` is the *post-write effective* state — i.e. it already includes
/// any pending Approval-typed nodes the caller plans to emit alongside
/// the transition.
pub fn check_guards(
    policy: &Policy,
    state: &ThreadState,
    from: &str,
    to: &str,
) -> Vec<GuardViolation> {
    let category = category_for_state(state);
    // Normalize 1.x state names (under-review, accepted, etc.) so legacy
    // policies and migrated chains line up with category-table keys that
    // use SPEC-3.0 canonical statuses.
    let from = normalize_state_name(from);
    let to = normalize_state_name(to);
    let transition = format!("{from}->{to}");
    let Some(rules) = policy.guards_for_transition(category, &transition) else {
        return Vec::new();
    };
    rules
        .iter()
        .filter_map(|rule| evaluate_rule(rule, state))
        .collect()
}

/// Evaluate a single guard rule against `state`. Returns `Some(violation)`
/// when the rule is not satisfied.
pub fn evaluate_rule(rule: &GuardRule, state: &ThreadState) -> Option<GuardViolation> {
    match rule {
        GuardRule::NoOpenObjections => {
            let open = state.open_objections();
            (!open.is_empty()).then(|| GuardViolation {
                rule: rule.to_string(),
                reason: format!("{} open objection(s)", open.len()),
            })
        }
        GuardRule::NoOpenActions => {
            let open = state.open_actions();
            (!open.is_empty()).then(|| GuardViolation {
                rule: rule.to_string(),
                reason: format!("{} open action(s)", open.len()),
            })
        }
        GuardRule::OneApproval => {
            // SPEC-3.0 §3.2: "At least one non-retracted `approval` node
            // exists on the thread, regardless of actor type."
            let has = state
                .nodes
                .iter()
                .any(|n| n.node_type == NodeType::Approval && !n.retracted);
            (!has).then(|| GuardViolation {
                rule: rule.to_string(),
                reason: "no approval recorded".into(),
            })
        }
        GuardRule::HasCommitEvidence => {
            let has = state
                .evidence_items
                .iter()
                .any(|e| e.kind == EvidenceKind::Commit);
            (!has).then(|| GuardViolation {
                rule: rule.to_string(),
                reason: "no commit evidence attached".into(),
            })
        }
    }
}

// ---------------------------------------------------------------------
// Lint
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    Note,
    Warn,
}

#[derive(Debug, Clone)]
pub struct LintDiag {
    pub level: LintLevel,
    pub message: String,
}

impl std::fmt::Display for LintDiag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = match self.level {
            LintLevel::Note => "NOTE",
            LintLevel::Warn => "WARN",
        };
        write!(f, "{prefix} {}", self.message)
    }
}

/// Emit advisory lint notes for a 3.0 policy. Most validation moved to
/// `validate_against_registry` (which fails the load); this returns
/// non-blocking notes only.
pub fn lint_policy(policy: &Policy) -> Vec<LintDiag> {
    let mut diags = Vec::new();
    for (cat_name, cat_policy) in &policy.categories {
        let no_rules = cat_policy.guards.is_empty()
            && cat_policy.creation.is_none()
            && cat_policy.revise.is_none()
            && cat_policy.evidence.is_none()
            && cat_policy.allowed_node_types.is_empty();
        if no_rules {
            diags.push(LintDiag {
                level: LintLevel::Note,
                message: format!("category {cat_name:?} has no rules configured"),
            });
        }
    }
    diags
}

// ---------------------------------------------------------------------
// Render policy show
// ---------------------------------------------------------------------

pub fn render_policy_show(policy: &Policy) -> String {
    let mut lines: Vec<String> = Vec::new();

    let mut category_names: Vec<&String> = policy.categories.keys().collect();
    category_names.sort();
    for cat_name in &category_names {
        let cat = &policy.categories[*cat_name];
        lines.push(format!("[categories.{cat_name}]"));

        if !cat.guards.is_empty() {
            lines.push("  guards:".into());
            let mut transitions: Vec<&String> = cat.guards.keys().collect();
            transitions.sort();
            for t in transitions {
                let rules: Vec<String> = cat.guards[t].iter().map(|r| r.to_string()).collect();
                lines.push(format!("    {t}: {}", rules.join(", ")));
            }
        }
        if let Some(c) = &cat.creation {
            lines.push("  creation:".into());
            lines.push(format!("    required_body: {}", c.required_body));
            if !c.body_sections.is_empty() {
                lines.push(format!(
                    "    body_sections: [{}]",
                    c.body_sections.join(", ")
                ));
            }
        }
        if !cat.allowed_node_types.is_empty() {
            lines.push("  allowed_node_types:".into());
            let mut statuses: Vec<&String> = cat.allowed_node_types.keys().collect();
            statuses.sort();
            for s in statuses {
                let kinds: Vec<String> = cat.allowed_node_types[s]
                    .iter()
                    .map(node_kind_str)
                    .collect();
                let body = if kinds.is_empty() {
                    "(none allowed)".to_string()
                } else {
                    kinds.join(", ")
                };
                lines.push(format!("    {s}: {body}"));
            }
        }
        if let Some(r) = &cat.revise {
            lines.push("  revise:".into());
            if let Some(list) = &r.allow_body_revise {
                lines.push(format!("    body: [{}]", list.join(", ")));
            }
            if let Some(list) = &r.allow_node_revise {
                lines.push(format!("    node: [{}]", list.join(", ")));
            }
        }
        if let Some(e) = &cat.evidence {
            lines.push("  evidence:".into());
            if let Some(list) = &e.allow_evidence {
                lines.push(format!("    allow: [{}]", list.join(", ")));
            }
        }
        lines.push(String::new());
    }

    lines.push("[checks]".into());
    lines.push(format!("  strict = {}", policy.checks.strict));

    lines.join("\n")
}

fn node_kind_str(k: &NodeKind) -> String {
    match k {
        NodeKind::Comment => "comment",
        NodeKind::Approval => "approval",
        NodeKind::Objection => "objection",
        NodeKind::Action => "action",
    }
    .to_string()
}

// ---------------------------------------------------------------------
// TERMINAL_STATES: kept for `commands::shortlog` (filters out done/etc).
// SPEC-3.0 union of rfc + task terminal statuses.
// ---------------------------------------------------------------------

pub const TERMINAL_STATES: &[&str] = &["done", "rejected", "deprecated", "withdrawn"];

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod category_registry_tests {
    use super::*;

    #[test]
    fn built_in_provides_rfc_and_task() {
        let r = CategoryRegistry::built_in();
        assert!(r.get("rfc").is_some());
        assert!(r.get("task").is_some());
    }

    #[test]
    fn rfc_initial_status_is_draft() {
        let r = CategoryRegistry::built_in();
        assert_eq!(r.get("rfc").unwrap().initial_status, "draft");
    }

    #[test]
    fn task_initial_status_is_open() {
        let r = CategoryRegistry::built_in();
        assert_eq!(r.get("task").unwrap().initial_status, "open");
    }

    #[test]
    fn rfc_draft_to_open_allowed() {
        let r = CategoryRegistry::built_in();
        assert!(r.get("rfc").unwrap().allows_transition("draft", "open"));
    }

    #[test]
    fn rfc_done_to_draft_disallowed() {
        let r = CategoryRegistry::built_in();
        assert!(!r.get("rfc").unwrap().allows_transition("done", "draft"));
    }

    #[test]
    fn validate_status_rejects_unknown_category() {
        let r = CategoryRegistry::built_in();
        let err = r.validate_status("bogus", "draft").unwrap_err();
        assert!(matches!(err, ForumError::SnapshotInvalid(_)));
    }

    #[test]
    fn validate_status_rejects_unknown_status() {
        let r = CategoryRegistry::built_in();
        let err = r.validate_status("rfc", "merged").unwrap_err();
        assert!(matches!(err, ForumError::SnapshotInvalid(_)));
    }

    #[test]
    fn validate_status_accepts_known_pair() {
        let r = CategoryRegistry::built_in();
        assert!(r.validate_status("rfc", "draft").is_ok());
        assert!(r.validate_status("task", "working").is_ok());
    }
}

#[cfg(test)]
mod legacy_detection_tests {
    use super::*;

    #[test]
    fn rejects_v2_array_of_tables_guards() {
        let toml = "[[guards]]\non = \"draft->open\"\nrequires = [\"no_open_objections\"]\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        assert!(hint.contains("[[guards]]"));
        assert!(hint.contains("§3.2"));
    }

    #[test]
    fn rejects_requires_field() {
        let toml = "[categories.rfc.guards]\nrequires = [\"no_open_objections\"]\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        assert!(hint.contains("requires ="));
    }

    #[test]
    fn rejects_v2_creation_rules_kind_keyed() {
        let toml = "[creation_rules.rfc]\nrequired_body = true\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        assert!(hint.contains("creation_rules"));
    }

    #[test]
    fn rejects_v2_creation_rules_facet_scoped() {
        let toml = "[creation_rules.proposal.tag.cross-cutting]\nrequired_body = true\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        assert!(hint.contains("creation_rules"));
    }

    #[test]
    fn rejects_v2_node_rules() {
        let toml = "[node_rules]\n\"draft\" = [\"comment\"]\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        assert!(hint.contains("node_rules"));
    }

    #[test]
    fn rejects_v2_revise_rules() {
        let toml = "[revise_rules]\nallow_body_revise = [\"draft\"]\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        assert!(hint.contains("revise_rules"));
    }

    #[test]
    fn rejects_v2_evidence_rules() {
        let toml = "[evidence_rules]\nallow_evidence = [\"draft\"]\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        assert!(hint.contains("evidence_rules"));
    }

    #[test]
    fn rejects_facet_scoped_guard_on_field() {
        let toml = "[[guards]]\non = \"lifecycle=proposal : review->done\"\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        // First match wins on the `[[guards]]` line.
        assert!(hint.contains("[[guards]]") || hint.contains("lifecycle"));
    }

    #[test]
    fn rejects_one_human_approval() {
        let toml = "[categories.rfc.guards]\n\"review->done\" = [\"one_human_approval\"]\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        assert!(hint.contains("one_human_approval"));
        assert!(hint.contains("one_approval"));
    }

    #[test]
    fn rejects_at_least_one_summary() {
        let toml = "[categories.rfc.guards]\n\"review->done\" = [\"at_least_one_summary\"]\n";
        let hint = detect_legacy_policy_form(toml).expect("should detect");
        assert!(hint.contains("at_least_one_summary"));
    }

    #[test]
    fn accepts_clean_v3_form() {
        let toml = r#"
[categories.rfc.guards]
"review->done" = ["one_approval", "no_open_objections"]

[categories.task.creation]
required_body = false
body_sections = ["Background", "Acceptance criteria"]

[checks]
strict = false
"#;
        assert!(detect_legacy_policy_form(toml).is_none());
    }

    #[test]
    fn ignores_comments_mentioning_removed_rules() {
        let toml = "# Note: at_least_one_summary was removed in 3.0.\n\
                    # Old form: [[guards]] requires = [...] — now category-keyed.\n\
                    [categories.rfc.guards]\n\"review->done\" = [\"one_approval\"]\n";
        assert!(detect_legacy_policy_form(toml).is_none());
    }
}

#[cfg(test)]
mod policy_load_tests {
    use super::*;

    fn write_temp(toml: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".forum/policy.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, toml).unwrap();
        (dir, path)
    }

    #[test]
    fn round_trip_clean_v3() {
        let toml = r#"
[categories.rfc.guards]
"review->done" = ["one_approval", "no_open_objections"]

[categories.task.guards]
"working->done" = ["no_open_actions"]

[categories.rfc.creation]
required_body = true
body_sections = ["Goal", "Non-goals"]

[checks]
strict = false
"#;
        let (_dir, path) = write_temp(toml);
        let policy = Policy::load(&path).unwrap();
        let rfc_guards = policy.guards_for_transition("rfc", "review->done").unwrap();
        assert_eq!(rfc_guards.len(), 2);
        assert_eq!(rfc_guards[0], GuardRule::OneApproval);
        let creation = policy.creation_rules_for("rfc").unwrap();
        assert!(creation.required_body);
        assert_eq!(creation.body_sections, vec!["Goal", "Non-goals"]);
        let task_guards = policy
            .guards_for_transition("task", "working->done")
            .unwrap();
        assert_eq!(task_guards, &[GuardRule::NoOpenActions]);
    }

    #[test]
    fn rejects_v2_array_of_tables_form() {
        let toml = "[[guards]]\non = \"draft->open\"\nrequires = [\"no_open_objections\"]\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("[[guards]]"), "got: {msg}");
        assert!(msg.contains("git forum migrate --to 3.0"), "got: {msg}");
    }

    #[test]
    fn rejects_v2_kind_keyed_creation_rules() {
        let toml = r#"
[creation_rules.rfc]
required_body = true

[creation_rules.task]
required_body = false
"#;
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("creation_rules"));
        assert!(msg.contains("git forum migrate --to 3.0"));
    }

    #[test]
    fn rejects_v2_facet_scoped_creation_rules() {
        let toml = "[creation_rules.proposal.tag.cross-cutting]\nrequired_body = true\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("creation_rules"));
    }

    #[test]
    fn rejects_v2_node_rules_table() {
        let toml = "[node_rules]\n\"draft\" = [\"comment\"]\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        assert!(err.to_string().contains("node_rules"));
    }

    #[test]
    fn rejects_v2_revise_rules_table() {
        let toml = "[revise_rules]\nallow_body_revise = [\"draft\"]\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        assert!(err.to_string().contains("revise_rules"));
    }

    #[test]
    fn rejects_v2_evidence_rules_table() {
        let toml = "[evidence_rules]\nallow_evidence = [\"draft\"]\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        assert!(err.to_string().contains("evidence_rules"));
    }

    #[test]
    fn rejects_one_human_approval_rule() {
        let toml = "[categories.rfc.guards]\n\"review->done\" = [\"one_human_approval\"]\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("one_human_approval"));
        assert!(msg.contains("one_approval"));
    }

    #[test]
    fn rejects_at_least_one_summary_rule() {
        let toml = "[categories.rfc.guards]\n\"review->done\" = [\"at_least_one_summary\"]\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        assert!(err.to_string().contains("at_least_one_summary"));
    }

    #[test]
    fn rejects_unknown_category() {
        // Non-built-in category without §3.1 definition fields → reject.
        let toml = "[categories.bogus.guards]\n\"a->b\" = []\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non-built-in category"), "got: {msg}");
        assert!(msg.contains("rfc, task"), "got: {msg}");
    }

    #[test]
    fn accepts_custom_category_with_full_definition() {
        // SPEC-3.0 §3.1: repos MAY define additional categories.
        let toml = r#"
[categories.spike]
initial_status = "draft"
statuses = ["draft", "investigating", "done"]
transitions = ["draft->investigating", "investigating->done"]

[categories.spike.guards]
"investigating->done" = ["one_approval"]
"#;
        let (_dir, path) = write_temp(toml);
        let policy = Policy::load(&path).expect("custom category accepted");
        let reg = policy.effective_registry();
        let spike = reg.get("spike").expect("custom category in registry");
        assert_eq!(spike.initial_status, "draft");
        assert!(spike.allows_transition("draft", "investigating"));
        // Built-ins survive alongside the custom category.
        assert!(reg.get("rfc").is_some());
        assert!(reg.get("task").is_some());
    }

    #[test]
    fn accepts_built_in_override() {
        // SPEC-3.0 §3.1: repos MAY override built-in category definitions.
        let toml = r#"
[categories.rfc]
initial_status = "open"
statuses = ["open", "review", "done"]
transitions = ["open->review", "review->done"]
"#;
        let (_dir, path) = write_temp(toml);
        let policy = Policy::load(&path).expect("built-in override accepted");
        let reg = policy.effective_registry();
        let rfc = reg.get("rfc").expect("rfc still present");
        assert_eq!(rfc.initial_status, "open");
        assert!(rfc.allows_transition("open", "review"));
        // Built-in `draft->open` is gone after override.
        assert!(!rfc.allows_transition("draft", "open"));
    }

    #[test]
    fn rejects_partial_definition_override() {
        // Partial override (only initial_status, no statuses/transitions) → reject.
        let toml = r#"
[categories.rfc]
initial_status = "open"
"#;
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("partial category definition"), "got: {msg}");
    }

    #[test]
    fn rejects_initial_status_not_in_statuses() {
        let toml = r#"
[categories.spike]
initial_status = "starting"
statuses = ["draft", "done"]
transitions = ["draft->done"]
"#;
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("initial_status"), "got: {msg}");
    }

    #[test]
    fn rejects_duplicate_transition() {
        let toml = r#"
[categories.spike]
initial_status = "draft"
statuses = ["draft", "done"]
transitions = ["draft->done", "draft->done"]
"#;
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        assert!(err.to_string().contains("duplicate transition"));
    }

    #[test]
    fn rejects_transition_with_status_outside_statuses() {
        let toml = r#"
[categories.spike]
initial_status = "draft"
statuses = ["draft", "done"]
transitions = ["draft->shipped"]
"#;
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("shipped"), "got: {msg}");
    }

    #[test]
    fn override_unblocks_previously_invalid_transition() {
        // The built-in rfc has no `open->done` edge. With an override that
        // adds it, the guard table is accepted instead of rejected.
        let toml = r#"
[categories.rfc]
initial_status = "draft"
statuses = ["draft", "open", "done"]
transitions = ["draft->open", "open->done"]

[categories.rfc.guards]
"open->done" = ["one_approval"]
"#;
        let (_dir, path) = write_temp(toml);
        let policy = Policy::load(&path).expect("override unblocks open->done");
        assert!(policy.guards_for_transition("rfc", "open->done").is_some());
    }

    #[test]
    fn round_trip_serialize_parse() {
        // SPEC-3.0 §3.1 round-trip: parse → serialize → parse must be
        // byte-stable on the structural fields (HashMap iteration is
        // non-deterministic in TOML emit, so we compare semantically).
        let toml = r#"
[categories.rfc]
initial_status = "draft"
statuses = ["draft", "open", "review", "done"]
transitions = ["draft->open", "open->review", "review->done"]

[categories.rfc.guards]
"review->done" = ["one_approval", "no_open_objections"]

[categories.rfc.creation]
required_body = true
body_sections = ["Goal", "Non-goals"]

[categories.rfc.revise]
allow_body_revise = ["draft", "open"]

[categories.rfc.evidence]
allow_evidence = ["draft", "open"]

[checks]
strict = true
"#;
        let (_dir, path) = write_temp(toml);
        let p1 = Policy::load(&path).unwrap();
        let serialized = toml::to_string(&p1).expect("policy must round-trip via toml::to_string");
        let dir = tempfile::tempdir().unwrap();
        let path2 = dir.path().join("rt.toml");
        std::fs::write(&path2, &serialized).unwrap();
        let p2 = Policy::load(&path2).expect("re-parse after serialize");

        // Definition fields preserved.
        let reg2 = p2.effective_registry();
        let rfc = reg2.get("rfc").unwrap();
        assert_eq!(rfc.initial_status, "draft");
        assert_eq!(rfc.statuses, vec!["draft", "open", "review", "done"]);
        // Guards preserved.
        let g = p2
            .guards_for_transition("rfc", "review->done")
            .expect("guards survive round-trip");
        assert!(g.contains(&GuardRule::OneApproval));
        assert!(g.contains(&GuardRule::NoOpenObjections));
        // Operation checks preserved.
        let creation = p2.creation_rules_for("rfc").unwrap();
        assert!(creation.required_body);
        assert_eq!(creation.body_sections, vec!["Goal", "Non-goals"]);
        let revise = p2.revise_rules_for("rfc").unwrap();
        assert_eq!(
            revise.allow_body_revise.as_deref(),
            Some(&["draft".into(), "open".into()][..])
        );
        // Top-level checks preserved.
        assert!(p2.checks.strict);
    }

    #[test]
    fn round_trip_preserves_empty_allow_lists() {
        // SPEC-3.0 §3.3: empty allow list = explicit deny-all. Round trip
        // must preserve `Some(vec![])` (not collapse it to `None`).
        let toml = r#"
[categories.rfc.revise]
allow_body_revise = []

[categories.rfc.evidence]
allow_evidence = []
"#;
        let (_dir, path) = write_temp(toml);
        let p1 = Policy::load(&path).unwrap();
        let revise = p1.revise_rules_for("rfc").unwrap();
        assert_eq!(revise.allow_body_revise.as_deref(), Some(&[][..]));

        let serialized = toml::to_string(&p1).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path2 = dir.path().join("rt.toml");
        std::fs::write(&path2, &serialized).unwrap();
        let p2 = Policy::load(&path2).unwrap();
        let revise = p2.revise_rules_for("rfc").unwrap();
        assert_eq!(revise.allow_body_revise.as_deref(), Some(&[][..]));
        let evidence = p2.evidence_rules_for("rfc").unwrap();
        assert_eq!(evidence.allow_evidence.as_deref(), Some(&[][..]));
    }

    #[test]
    fn rejects_status_outside_category() {
        let toml = "[categories.rfc.guards]\n\"working->done\" = []\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("working"));
        assert!(msg.contains("statuses"));
    }

    #[test]
    fn rejects_invalid_transition_within_category() {
        // `done->review` exists in neither rfc nor task transitions.
        let toml = "[categories.rfc.guards]\n\"done->review\" = []\n";
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not a valid transition"), "got: {msg}");
    }

    #[test]
    fn rejects_revise_status_outside_category() {
        let toml = r#"
[categories.rfc.revise]
allow_body_revise = ["working"]
"#;
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("working"));
    }

    #[test]
    fn rejects_evidence_status_outside_category() {
        let toml = r#"
[categories.task.evidence]
allow_evidence = ["draft"]
"#;
        let (_dir, path) = write_temp(toml);
        let err = Policy::load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("draft"));
    }
}

#[cfg(test)]
mod evaluate_tests {
    use super::*;
    use crate::internal::thread::ThreadKind;

    fn approval_node(actor: &str) -> crate::internal::node::Node {
        crate::internal::node::Node {
            node_id: format!("approval-{actor}"),
            node_type: NodeType::Approval,
            actor: actor.into(),
            ..Default::default()
        }
    }

    fn objection_node() -> crate::internal::node::Node {
        crate::internal::node::Node {
            node_id: "obj1".into(),
            node_type: NodeType::Objection,
            ..Default::default()
        }
    }

    fn action_node() -> crate::internal::node::Node {
        crate::internal::node::Node {
            node_id: "act1".into(),
            node_type: NodeType::Action,
            ..Default::default()
        }
    }

    fn state_for(kind: ThreadKind) -> ThreadState {
        ThreadState {
            kind,
            lifecycle: kind.lifecycle(),
            ..Default::default()
        }
    }

    #[test]
    fn one_approval_accepts_any_actor_type() {
        // SPEC-3.0 §3.2: regardless of actor type.
        let mut state = state_for(ThreadKind::Rfc);
        state.nodes.push(approval_node("ai/codex"));
        assert!(evaluate_rule(&GuardRule::OneApproval, &state).is_none());

        let mut state2 = state_for(ThreadKind::Rfc);
        state2.nodes.push(approval_node("human/alice"));
        assert!(evaluate_rule(&GuardRule::OneApproval, &state2).is_none());
    }

    #[test]
    fn one_approval_fails_when_no_approvals() {
        let state = state_for(ThreadKind::Rfc);
        let v = evaluate_rule(&GuardRule::OneApproval, &state).unwrap();
        assert_eq!(v.rule, "one_approval");
    }

    #[test]
    fn no_open_objections_passes_when_thread_clean() {
        let state = state_for(ThreadKind::Rfc);
        assert!(evaluate_rule(&GuardRule::NoOpenObjections, &state).is_none());
    }

    #[test]
    fn no_open_objections_fails_when_open_objection_present() {
        let mut state = state_for(ThreadKind::Rfc);
        state.nodes.push(objection_node());
        let v = evaluate_rule(&GuardRule::NoOpenObjections, &state).unwrap();
        assert!(v.reason.contains("1 open objection"));
    }

    #[test]
    fn no_open_actions_fails_when_open_action_present() {
        let mut state = state_for(ThreadKind::Issue);
        state.nodes.push(action_node());
        let v = evaluate_rule(&GuardRule::NoOpenActions, &state).unwrap();
        assert!(v.reason.contains("1 open action"));
    }

    #[test]
    fn check_guards_empty_when_no_rules_for_transition() {
        let policy = Policy::default();
        let state = state_for(ThreadKind::Rfc);
        let v = check_guards(&policy, &state, "draft", "open");
        assert!(v.is_empty());
    }

    #[test]
    fn check_guards_runs_rules_for_matching_category_transition() {
        let mut policy = Policy::default();
        let mut rfc = CategoryPolicy::default();
        rfc.guards.insert(
            "review->done".into(),
            vec![GuardRule::OneApproval, GuardRule::NoOpenObjections],
        );
        policy.categories.insert("rfc".into(), rfc);

        let mut state = state_for(ThreadKind::Rfc);
        state.nodes.push(objection_node());
        let v = check_guards(&policy, &state, "review", "done");
        assert_eq!(v.len(), 2);
        let rules: Vec<&str> = v.iter().map(|x| x.rule.as_str()).collect();
        assert!(rules.contains(&"one_approval"));
        assert!(rules.contains(&"no_open_objections"));
    }
}
