use std::collections::HashMap;

use serde::Deserialize;

use super::error::{ForumError, ForumResult};
use super::event;
use super::event::{Lifecycle, NodeType};
use super::evidence::EvidenceKind;
use super::lint_emit::{self, LintEmitter};
use super::thread::ThreadState;

/// SPEC-2.0 §7.1 — boolean predicate over `lifecycle=...` and `tag=...`
/// terms. Drives guard scoping (`on = "lifecycle=proposal AND tag=cross-cutting : review->done"`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FacetPredicate {
    /// Always matches (unscoped guard).
    #[default]
    Always,
    Lifecycle(Lifecycle),
    Tag(String),
    And(Box<FacetPredicate>, Box<FacetPredicate>),
    Or(Box<FacetPredicate>, Box<FacetPredicate>),
    Not(Box<FacetPredicate>),
}

impl FacetPredicate {
    /// Whether this predicate matches the given thread state.
    pub fn matches(&self, state: &ThreadState) -> bool {
        match self {
            Self::Always => true,
            Self::Lifecycle(l) => state.lifecycle == *l,
            Self::Tag(t) => state.tags.iter().any(|s| s == t),
            Self::And(a, b) => a.matches(state) && b.matches(state),
            Self::Or(a, b) => a.matches(state) || b.matches(state),
            Self::Not(inner) => !inner.matches(state),
        }
    }
}

/// Recursive-descent parser for the facet expression grammar:
///
/// ```text
/// expr     := term (OR term)*
/// term     := factor (AND factor)*
/// factor   := NOT factor | '(' expr ')' | predicate
/// predicate := ('lifecycle' '=' value | 'tag' '=' value)
/// value    := [a-z][a-z0-9-]*
/// ```
///
/// Whitespace is insignificant. `AND` / `OR` / `NOT` are case-insensitive.
fn parse_facet_predicate(input: &str) -> Result<FacetPredicate, String> {
    let tokens = tokenize_facet(input)?;
    let mut cursor = 0usize;
    let pred = parse_or(&tokens, &mut cursor)?;
    if cursor != tokens.len() {
        return Err(format!(
            "trailing tokens in facet predicate: {:?}",
            &tokens[cursor..]
        ));
    }
    Ok(pred)
}

#[derive(Debug, Clone, PartialEq)]
enum FacetTok {
    Ident(String),
    Eq,
    LParen,
    RParen,
    And,
    Or,
    Not,
}

fn tokenize_facet(input: &str) -> Result<Vec<FacetTok>, String> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' => {
                chars.next();
            }
            '(' => {
                chars.next();
                out.push(FacetTok::LParen);
            }
            ')' => {
                chars.next();
                out.push(FacetTok::RParen);
            }
            '=' => {
                chars.next();
                out.push(FacetTok::Eq);
            }
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => {
                let mut buf = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        buf.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let upper = buf.to_ascii_uppercase();
                out.push(match upper.as_str() {
                    "AND" => FacetTok::And,
                    "OR" => FacetTok::Or,
                    "NOT" => FacetTok::Not,
                    _ => FacetTok::Ident(buf),
                });
            }
            other => return Err(format!("unexpected character {other:?} in facet predicate")),
        }
    }
    Ok(out)
}

fn parse_or(toks: &[FacetTok], i: &mut usize) -> Result<FacetPredicate, String> {
    let mut left = parse_and(toks, i)?;
    while matches!(toks.get(*i), Some(FacetTok::Or)) {
        *i += 1;
        let right = parse_and(toks, i)?;
        left = FacetPredicate::Or(Box::new(left), Box::new(right));
    }
    Ok(left)
}

fn parse_and(toks: &[FacetTok], i: &mut usize) -> Result<FacetPredicate, String> {
    let mut left = parse_factor(toks, i)?;
    while matches!(toks.get(*i), Some(FacetTok::And)) {
        *i += 1;
        let right = parse_factor(toks, i)?;
        left = FacetPredicate::And(Box::new(left), Box::new(right));
    }
    Ok(left)
}

fn parse_factor(toks: &[FacetTok], i: &mut usize) -> Result<FacetPredicate, String> {
    match toks.get(*i) {
        Some(FacetTok::Not) => {
            *i += 1;
            let inner = parse_factor(toks, i)?;
            Ok(FacetPredicate::Not(Box::new(inner)))
        }
        Some(FacetTok::LParen) => {
            *i += 1;
            let expr = parse_or(toks, i)?;
            match toks.get(*i) {
                Some(FacetTok::RParen) => {
                    *i += 1;
                    Ok(expr)
                }
                _ => Err("missing ')' in facet predicate".into()),
            }
        }
        Some(FacetTok::Ident(name)) => {
            let key = name.clone();
            *i += 1;
            match toks.get(*i) {
                Some(FacetTok::Eq) => {
                    *i += 1;
                }
                _ => return Err(format!("expected '=' after {key:?} in facet predicate")),
            }
            let value = match toks.get(*i) {
                Some(FacetTok::Ident(v)) => v.clone(),
                other => return Err(format!("expected identifier after {key:?}=, got {other:?}")),
            };
            *i += 1;
            match key.as_str() {
                "lifecycle" => Lifecycle::parse(&value)
                    .ok_or_else(|| format!("unknown lifecycle {value:?}"))
                    .map(FacetPredicate::Lifecycle),
                "tag" => Ok(FacetPredicate::Tag(value)),
                other => Err(format!(
                    "unknown facet key {other:?} (expected `lifecycle` or `tag`)"
                )),
            }
        }
        other => Err(format!("unexpected token {other:?} in facet predicate")),
    }
}

/// A named guard rule.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardRule {
    NoOpenObjections,
    NoOpenActions,
    AtLeastOneSummary,
    OneHumanApproval,
    HasCommitEvidence,
}

impl std::fmt::Display for GuardRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOpenObjections => write!(f, "no_open_objections"),
            Self::NoOpenActions => write!(f, "no_open_actions"),
            Self::AtLeastOneSummary => write!(f, "at_least_one_summary"),
            Self::OneHumanApproval => write!(f, "one_human_approval"),
            Self::HasCommitEvidence => write!(f, "has_commit_evidence"),
        }
    }
}

/// A guard entry: a set of rules that must pass for a given transition.
///
/// SPEC-2.0 §7.1: the `on` field accepts a facet expression scope plus a
/// transition, separated by `:`. Examples:
/// - `"review->done"` (unscoped — applies to any thread)
/// - `"lifecycle=proposal AND tag=cross-cutting : review->done"` (scoped)
///
/// Legacy 1.x kind-prefixed form (`"rfc:under-review->accepted"`) is
/// accepted at load time and auto-translated to a `Lifecycle(...)`
/// predicate with a one-shot warning per rewrite.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GuardEntry {
    pub on: String,
    pub requires: Vec<GuardRule>,
    /// SPEC-2.0 §7.1 facet predicate (Always = unscoped).
    #[serde(skip)]
    pub predicate: FacetPredicate,
    /// Parsed transition string ("from->to"), without the predicate.
    #[serde(skip)]
    pub transition: String,
}

/// Creation rules — required body / required body sections (SPEC-2.0 §7.2).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreationRules {
    #[serde(default)]
    pub required_body: bool,
    #[serde(default)]
    pub body_sections: Vec<String>,
}

/// Per-lifecycle creation rules with optional per-tag specialization.
///
/// SPEC-2.0 §7.2: `creation_rules.<lifecycle>` carries the base rules;
/// `creation_rules.<lifecycle>.tag.<name>` overrides them for threads
/// carrying that tag. Resolution: most-specific match wins. Multi-tag
/// combiners are deferred per §7.2.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LifecycleCreationRules {
    #[serde(flatten)]
    pub base: CreationRules,
    #[serde(default)]
    pub tag: HashMap<String, CreationRules>,
}

/// Rules controlling which states allow body/node revisions.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReviseRules {
    #[serde(default)]
    pub allow_body_revise: Vec<String>,
    #[serde(default)]
    pub allow_node_revise: Vec<String>,
}

/// Rules controlling which states allow evidence addition.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EvidenceRules {
    #[serde(default)]
    pub allow_evidence: Vec<String>,
}

/// Global check settings.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChecksConfig {
    #[serde(default)]
    pub strict: bool,
}

/// Parsed policy loaded from `.forum/policy.toml`.
///
/// Preconditions: none (loaded from file).
/// Postconditions: guards are populated from TOML.
/// Failure modes: ForumError::Config on parse failure.
/// Side effects: none (read-only after load).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Policy {
    #[serde(default, rename = "guards")]
    pub guards: Vec<GuardEntry>,
    /// Keyed by lifecycle name (`proposal` / `execution` / `record`).
    /// Legacy 1.x kind keys (`rfc` / `issue` / `dec` / `task`) are
    /// auto-translated at load time per SPEC-2.0 §2.3.3.
    #[serde(default)]
    pub creation_rules: HashMap<String, LifecycleCreationRules>,
    #[serde(default)]
    pub node_rules: HashMap<String, Vec<NodeType>>,
    #[serde(default)]
    pub revise_rules: Option<ReviseRules>,
    #[serde(default)]
    pub evidence_rules: Option<EvidenceRules>,
    #[serde(default)]
    pub checks: ChecksConfig,
}

impl Policy {
    /// Load and parse policy from the given path. Legacy `kind:`-scoped
    /// guards auto-rewrite to `lifecycle=` predicates; the rewrites are
    /// printed to stderr so users see the deprecation (SPEC-2.0 §10.4).
    ///
    /// Lint warnings are routed through the process-wide [`LintEmitter`]
    /// (see `internal::lint_emit`), which throttles repeats and renders
    /// paths repo-relative. Tests that need an isolated emitter call
    /// [`Policy::load_with_emitter`] instead.
    pub fn load(path: &std::path::Path) -> ForumResult<Self> {
        Self::load_with_emitter(path, lint_emit::current())
    }

    /// Like [`Policy::load`], but emits warnings through the supplied
    /// emitter. Use this in tests to capture or fully suppress lint
    /// output without touching the global emitter.
    pub fn load_with_emitter(path: &std::path::Path, emitter: &LintEmitter) -> ForumResult<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ForumError::Config(format!("cannot read policy.toml: {e}")))?;
        // ADR-006 Consequences: scan for the removed `at_least_one_summary`
        // predicate before parsing so we can name the file + line of every
        // mention. Emit warnings irrespective of whether the parse succeeds.
        for hit in scan_at_least_one_summary_warnings(&text) {
            emitter.emit(
                "at_least_one_summary",
                Some(path),
                Some(hit.line),
                AT_LEAST_ONE_SUMMARY_BODY,
            );
        }
        let mut policy: Self = toml::from_str(&text)
            .map_err(|e| ForumError::Config(format!("invalid policy.toml: {e}")))?;
        // 1.x → 2.0 shape rewrites (predicate strip, guard scopes,
        // creation rules) all live in `compat::v1`; they emit deprecation
        // warnings through the supplied emitter.
        super::compat::v1::rewrite_legacy_policy(&mut policy, emitter, path);
        Ok(policy)
    }

    /// Resolve creation rules for a given lifecycle + tag set, applying
    /// most-specific-match (SPEC-2.0 §7.2): a tag rule overrides the base
    /// lifecycle rule when the thread carries that tag. Multi-tag ties are
    /// broken alphabetically per the spec's "MAY pick deterministically"
    /// allowance.
    pub fn resolve_creation_rules(
        &self,
        lifecycle: Lifecycle,
        tags: &[String],
    ) -> Option<&CreationRules> {
        let entry = self.creation_rules.get(lifecycle.as_str())?;
        let mut matching_tags: Vec<&String> =
            tags.iter().filter(|t| entry.tag.contains_key(*t)).collect();
        matching_tags.sort();
        if let Some(t) = matching_tags.first() {
            return entry.tag.get(*t);
        }
        Some(&entry.base)
    }

    /// Return guards whose transition matches `transition` and whose
    /// facet predicate matches `state`. The query is normalized so
    /// callers can pass either 1.x or 2.0 state names.
    pub fn guards_for(&self, transition: &str, state: &ThreadState) -> Vec<&GuardEntry> {
        let normalized = normalize_transition_str(transition);
        self.guards
            .iter()
            .filter(|g| g.transition == normalized && g.predicate.matches(state))
            .collect()
    }
}

/// A guard violation: a rule that was not satisfied.
#[derive(Debug, Clone)]
pub struct GuardViolation {
    pub rule: String,
    pub reason: String,
}

/// Evaluate all guards for a transition and return any violations.
///
/// Preconditions: `state` is the *post-write effective* state — i.e. it
/// already includes any pending Approval-typed nodes the caller plans to
/// emit alongside the transition (see SPEC-2.0 §2.8).
/// Postconditions: empty vec means all guards pass.
/// Failure modes: none (returns violations, not errors).
/// Side effects: none.
pub fn check_guards(
    policy: &Policy,
    state: &ThreadState,
    from: &str,
    to: &str,
) -> Vec<GuardViolation> {
    let transition = format!("{from}->{to}");
    let mut violations = Vec::new();
    for guard in policy.guards_for(&transition, state) {
        for rule in &guard.requires {
            if let Some(v) = evaluate_rule(rule, state) {
                violations.push(v);
            }
        }
    }
    violations
}

/// Evaluate a single guard rule. Returns `Some(violation)` if the rule is not satisfied.
pub fn evaluate_rule(rule: &GuardRule, state: &ThreadState) -> Option<GuardViolation> {
    match rule {
        GuardRule::NoOpenObjections => {
            let open = state.open_objections();
            if !open.is_empty() {
                Some(GuardViolation {
                    rule: rule.to_string(),
                    reason: format!("{} open objection(s)", open.len()),
                })
            } else {
                None
            }
        }
        GuardRule::NoOpenActions => {
            let open = state.open_actions();
            if !open.is_empty() {
                Some(GuardViolation {
                    rule: rule.to_string(),
                    reason: format!("{} open action(s)", open.len()),
                })
            } else {
                None
            }
        }
        GuardRule::AtLeastOneSummary => {
            // ADR-006: predicate removed in 2.0. `Policy::load` strips
            // these entries from `requires` and warns at load time. If
            // a guard somehow still carries one (e.g. constructed
            // programmatically), treat it as a no-op rather than
            // half-enforcing a removed rule.
            None
        }
        GuardRule::OneHumanApproval => {
            // SPEC-2.0 §2.8: approvals are `approval`-typed nodes. Count any
            // non-retracted approval node whose actor is a human.
            let has_human = state.nodes.iter().any(|n| {
                n.node_type == NodeType::Approval && !n.retracted && n.actor.starts_with("human/")
            });
            if !has_human {
                Some(GuardViolation {
                    rule: rule.to_string(),
                    reason: "no human approval recorded".into(),
                })
            } else {
                None
            }
        }
        GuardRule::HasCommitEvidence => {
            let has_commit = state
                .evidence_items
                .iter()
                .any(|e| e.kind == EvidenceKind::Commit);
            if !has_commit {
                Some(GuardViolation {
                    rule: rule.to_string(),
                    reason: "no commit evidence attached".into(),
                })
            } else {
                None
            }
        }
    }
}

/// Severity level for a lint diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    /// Informational note (e.g. multi-kind transition — intentional but worth knowing).
    Note,
    /// Warning about a likely mistake (e.g. unknown state name).
    Warn,
}

/// A single lint diagnostic with a severity level.
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

/// Lint a policy for structural problems.
///
/// Preconditions: policy is loaded.
/// Postconditions: returns a list of diagnostics (empty = OK).
/// Failure modes: none.
/// Side effects: none.
pub fn lint_policy(policy: &Policy) -> Vec<LintDiag> {
    let mut diags = Vec::new();
    let all_lifecycles = [Lifecycle::Proposal, Lifecycle::Execution, Lifecycle::Record];

    // SPEC-2.0 §3.1: known states are the union of the unified 2.0 graph
    // plus the 1.x state names that the boundary normalizer recognizes
    // (so existing kind-keyed policy.toml fixtures still pass lint).
    let all_states: std::collections::HashSet<&str> = event::unified_transitions()
        .iter()
        .flat_map(|(from, to)| [*from, *to])
        .chain([
            "proposed",
            "under-review",
            "reviewing",
            "accepted",
            "closed",
            "pending",
            "designing",
            "implementing",
        ])
        .collect();

    for guard in &policy.guards {
        // Parse the facet predicate (and surface unknown 1.x kind prefixes
        // / facet-syntax errors as warnings).
        let parsed = parse_guard_on(&guard.on);
        let (predicate, transition_part) = match parsed {
            Ok((pred, transition, _warning)) => (pred, transition),
            Err(msg) => {
                diags.push(LintDiag {
                    level: LintLevel::Warn,
                    message: format!("guard {:?}: {msg}", guard.on),
                });
                // Try to keep going so we still validate the transition
                // string when the scope failed.
                let transition = guard
                    .on
                    .split_once(':')
                    .map(|(_, t)| t)
                    .unwrap_or(&guard.on);
                (FacetPredicate::Always, transition.to_string())
            }
        };

        if !transition_part.contains("->") {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard 'on' field {:?} is not a valid transition (expected 'from->to')",
                    guard.on
                ),
            });
            continue;
        }

        let parts: Vec<&str> = transition_part.trim().splitn(2, "->").collect();
        let (from, to) = (parts[0].trim(), parts[1].trim());

        // Check for undefined states.
        if !all_states.contains(from) {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard {:?}: unknown state {:?}; valid states are: {}",
                    guard.on,
                    from,
                    sorted_states(&all_states),
                ),
            });
        }
        if !all_states.contains(to) {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard {:?}: unknown state {:?}; valid states are: {}",
                    guard.on,
                    to,
                    sorted_states(&all_states),
                ),
            });
        }

        // Lifecycle-pinned predicates: validate transition reachability.
        if let FacetPredicate::Lifecycle(lifecycle) = &predicate {
            if all_states.contains(from)
                && all_states.contains(to)
                && !event::is_valid_transition(*lifecycle, from, to)
            {
                diags.push(LintDiag {
                    level: LintLevel::Warn,
                    message: format!(
                        "guard {:?}: not a valid transition for {lifecycle}",
                        guard.on,
                    ),
                });
            }
            continue;
        }
        // Compound or tag-only predicates: skip the multi-kind check —
        // the predicate itself disambiguates.
        if !matches!(predicate, FacetPredicate::Always) {
            continue;
        }

        // Unscoped guard: emit a Note when the transition applies to
        // multiple lifecycles (the user may want a `lifecycle=` predicate
        // to disambiguate per §7.1) or warn when it applies to none.
        let matching_lifecycles: Vec<&str> = all_lifecycles
            .iter()
            .filter(|l| event::is_valid_transition(**l, from, to))
            .map(|l| l.as_str())
            .collect();
        if matching_lifecycles.len() > 1 {
            diags.push(LintDiag {
                level: LintLevel::Note,
                message: format!(
                    "guard {:?}: transition applies to multiple lifecycles ({}); \
                     scope with a `lifecycle=` predicate (SPEC-2.0 §7.1) if you need \
                     different rules per lifecycle",
                    guard.on,
                    matching_lifecycles.join(", "),
                ),
            });
        } else if matching_lifecycles.is_empty()
            && all_states.contains(from)
            && all_states.contains(to)
        {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard {:?}: not a valid transition for any lifecycle",
                    guard.on,
                ),
            });
        }
    }

    // Validate state names in operation checks.
    lint_state_list(
        &mut diags,
        &all_states,
        "revise_rules.allow_body_revise",
        policy
            .revise_rules
            .as_ref()
            .map(|r| r.allow_body_revise.as_slice())
            .unwrap_or(&[]),
    );
    lint_state_list(
        &mut diags,
        &all_states,
        "revise_rules.allow_node_revise",
        policy
            .revise_rules
            .as_ref()
            .map(|r| r.allow_node_revise.as_slice())
            .unwrap_or(&[]),
    );
    lint_state_list(
        &mut diags,
        &all_states,
        "evidence_rules.allow_evidence",
        policy
            .evidence_rules
            .as_ref()
            .map(|r| r.allow_evidence.as_slice())
            .unwrap_or(&[]),
    );

    // Semantic check: warn when an allow-list misses all non-terminal
    // states for a lifecycle.
    lint_allow_list_coverage(
        &mut diags,
        "revise_rules.allow_body_revise",
        policy
            .revise_rules
            .as_ref()
            .map(|r| r.allow_body_revise.as_slice())
            .unwrap_or(&[]),
        &all_lifecycles,
    );
    lint_allow_list_coverage(
        &mut diags,
        "revise_rules.allow_node_revise",
        policy
            .revise_rules
            .as_ref()
            .map(|r| r.allow_node_revise.as_slice())
            .unwrap_or(&[]),
        &all_lifecycles,
    );
    lint_allow_list_coverage(
        &mut diags,
        "evidence_rules.allow_evidence",
        policy
            .evidence_rules
            .as_ref()
            .map(|r| r.allow_evidence.as_slice())
            .unwrap_or(&[]),
        &all_lifecycles,
    );

    // Validate state names in node_rules keys.
    for state_name in policy.node_rules.keys() {
        if !all_states.contains(state_name.as_str()) {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "node_rules: unknown state {:?}; valid states are: {}",
                    state_name,
                    sorted_states(&all_states),
                ),
            });
        }
    }

    // @ltojzq9l: warn when an allow-list carries 1.x state names without
    // their 2.0 canonical equivalents. Stored thread state is always
    // canonical (per state_change), so a legacy-only entry is dead weight
    // even after runtime normalization — surfaces as user-visible drift.
    lint_legacy_state_names(
        &mut diags,
        "evidence_rules.allow_evidence",
        policy
            .evidence_rules
            .as_ref()
            .map(|r| r.allow_evidence.as_slice())
            .unwrap_or(&[]),
    );
    if let Some(revise) = &policy.revise_rules {
        lint_legacy_state_names(
            &mut diags,
            "revise_rules.allow_body_revise",
            &revise.allow_body_revise,
        );
        lint_legacy_state_names(
            &mut diags,
            "revise_rules.allow_node_revise",
            &revise.allow_node_revise,
        );
    }
    let node_rules_keys: Vec<String> = policy.node_rules.keys().cloned().collect();
    lint_legacy_state_names(&mut diags, "node_rules", &node_rules_keys);

    diags
}

/// SPEC-2.0 §3.1.1 / @ltojzq9l: state-name allow-lists in policy should
/// use 2.0 canonical names. A legacy 1.x name (`under-review`,
/// `reviewing`, `closed`, `accepted`, `pending`, `designing`,
/// `implementing`, `proposed`) without its canonical sibling produces
/// drift between the policy display and stored state — runtime
/// tolerates the mismatch (operation_check::allow_list_contains) but
/// users still read confusing hints. Warn at lint time so the drift is
/// visible before runtime.
fn lint_legacy_state_names(diags: &mut Vec<LintDiag>, location: &str, entries: &[String]) {
    use std::collections::HashSet;
    let listed: HashSet<&str> = entries.iter().map(|s| s.as_str()).collect();
    for entry in entries {
        let canon = event::normalize_state_name(entry.as_str());
        if canon != entry.as_str() && !listed.contains(canon) {
            // entry is a legacy name AND its 2.0 form isn't also listed.
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "{location}: legacy 1.x state name {entry:?}; \
                     replace with the 2.0 canonical {canon:?} \
                     (or list both during migration). State events store \
                     canonical names, so the legacy entry alone produces \
                     contradictory error hints in operation checks."
                ),
            });
        }
    }
}

fn lint_state_list(
    diags: &mut Vec<LintDiag>,
    all_states: &std::collections::HashSet<&str>,
    field: &str,
    states: &[String],
) {
    for s in states {
        if !all_states.contains(s.as_str()) {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "{field}: unknown state {:?}; valid states are: {}",
                    s,
                    sorted_states(all_states),
                ),
            });
        }
    }
}

fn sorted_states(states: &std::collections::HashSet<&str>) -> String {
    let mut v: Vec<&str> = states.iter().copied().collect();
    v.sort_unstable();
    v.join(", ")
}

fn format_creation_rules(rules: &CreationRules) -> String {
    let mut parts: Vec<String> = Vec::new();
    if rules.required_body {
        parts.push("required_body=true".into());
    }
    if !rules.body_sections.is_empty() {
        parts.push(format!("sections=[{}]", rules.body_sections.join(", ")));
    }
    if parts.is_empty() {
        "(no restrictions)".to_string()
    } else {
        parts.join(", ")
    }
}

/// Terminal states are resolution/conclusion states where a thread's primary work is done.
/// Includes both 2.0 names and the 1.x names still produced by legacy fixtures.
pub const TERMINAL_STATES: &[&str] = &[
    "done",
    "rejected",
    "deprecated",
    "withdrawn",
    "closed",
    "accepted",
];

/// SPEC-2.0 §3.1.1: non-terminal states for a lifecycle (states where
/// active work happens). Derived from the unified graph filtered by
/// the lifecycle's allowed-state set.
fn non_terminal_states(lifecycle: Lifecycle) -> Vec<&'static str> {
    let mut states: std::collections::BTreeSet<&str> = event::unified_transitions()
        .iter()
        .flat_map(|(from, to)| [*from, *to])
        .filter(|s| lifecycle.allows_state(s) && !TERMINAL_STATES.contains(s))
        .collect();
    states.insert(lifecycle.initial_state());
    states.into_iter().collect()
}

/// Warn when an allow-list has no non-terminal states for a lifecycle.
fn lint_allow_list_coverage(
    diags: &mut Vec<LintDiag>,
    field: &str,
    states: &[String],
    all_lifecycles: &[Lifecycle],
) {
    if states.is_empty() {
        return;
    }
    let state_set: std::collections::HashSet<&str> = states.iter().map(|s| s.as_str()).collect();

    for lifecycle in all_lifecycles {
        let non_terminal = non_terminal_states(*lifecycle);
        let has_any = non_terminal.iter().any(|s| state_set.contains(*s));
        if !has_any {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "{field}: no states for {lifecycle} workflows; consider adding: {}",
                    non_terminal.join(", "),
                ),
            });
        }
    }
}

/// Parse the `on` field of a guard into a (predicate, transition, optional
/// warning). Formats accepted (SPEC-2.0 §7.1 + 1.x compat):
///
/// - `"from->to"` → unscoped (predicate = Always).
/// - `"<facet-expr> : from->to"` → scoped by facet predicate.
/// - `"<kind>:from->to"` (1.x) → auto-translated to
///   `lifecycle=<kind.lifecycle()>`, with a deprecation warning.
///
/// Returns `Err` only when a `<facet-expr>` portion is present but
/// fails to parse.
/// Parse a guard's `on` field into a (predicate, transition, optional
/// rewrite-warning) triple. Legacy kind-prefixed scopes route through
/// [`super::compat::v1::legacy_kind_prefix_to_lifecycle`] for the
/// `kind:from->to` → `lifecycle=...` rewrite.
///
/// Visibility: crate-internal so [`super::compat::v1::rewrite_legacy_policy`]
/// can dispatch through it without re-exporting the parser pipeline.
pub(crate) fn parse_guard_on(on: &str) -> Result<(FacetPredicate, String, Option<String>), String> {
    let Some((scope, transition)) = on.split_once(':') else {
        return Ok((FacetPredicate::Always, on.to_string(), None));
    };
    let scope_trimmed = scope.trim();
    // 1.x compat: a bare kind name as the prefix translates to a
    // `lifecycle=<...>` predicate. Detected by being a single alphanum
    // word with no `=` and matching one of the four legacy kinds.
    if !scope_trimmed.contains('=') && !scope_trimmed.contains(' ') && !scope_trimmed.contains('(')
    {
        if let Some(lifecycle) = super::compat::v1::legacy_kind_prefix_to_lifecycle(scope_trimmed) {
            let warning = format!(
                "guard {:?}: legacy kind-prefixed scope `{scope_trimmed}:` rewritten to \
                 `lifecycle={lifecycle}` (SPEC-2.0 §7.1 / §10.4)",
                on,
            );
            return Ok((
                FacetPredicate::Lifecycle(lifecycle),
                transition.to_string(),
                Some(warning),
            ));
        }
        // Unrecognized bare prefix: surface as parse error so lint
        // reports it (matches pre-2.0 behavior).
        return Err(format!("unknown kind prefix {scope_trimmed:?}"));
    }
    let predicate = parse_facet_predicate(scope_trimmed)?;
    Ok((predicate, transition.to_string(), None))
}

/// One occurrence of the removed `at_least_one_summary` predicate.
/// Path display is the emitter's job (see `internal::lint_emit`); the
/// scanner only reports `(line, _)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtLeastOneSummaryHit {
    /// 1-based line number of the offending occurrence.
    pub line: usize,
}

/// Body of the `at_least_one_summary` deprecation warning. Lifted out of
/// `scan_at_least_one_summary_warnings` so the file:line prefix can be
/// formatted by the emitter and tests can assert against the body
/// independently of path display.
pub const AT_LEAST_ONE_SUMMARY_BODY: &str =
    "predicate `at_least_one_summary` is removed (ADR-006); \
     it no longer fires. Either delete the predicate or require a non-empty \
     `body_sections` entry on the relevant `creation_rules`.";

/// ADR-006 / SPEC-2.0 §7.1: scan a policy.toml text for occurrences of
/// the removed `at_least_one_summary` predicate. Returns one hit per
/// non-comment line that mentions the predicate.
pub fn scan_at_least_one_summary_warnings(text: &str) -> Vec<AtLeastOneSummaryHit> {
    let mut hits = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        // Skip TOML comment lines: users may leave a deprecation note
        // ("# at_least_one_summary was removed per ADR-006") and that is
        // not the predicate firing.
        if line.trim_start().starts_with('#') {
            continue;
        }
        if line.contains("at_least_one_summary") {
            hits.push(AtLeastOneSummaryHit { line: idx + 1 });
        }
    }
    hits
}

/// Normalize the `from->to` portion of a guard transition string to 2.0
/// state names. Trims surrounding whitespace so guards written with
/// spaces around the `:` separator (`lifecycle=X : from->to`) match
/// query strings produced by the state machine.
///
/// Visibility: crate-internal so [`super::compat::v1::rewrite_legacy_policy`]
/// shares the same canonicalisation as `guards_for`.
pub(crate) fn normalize_transition_str(transition: &str) -> String {
    let trimmed = transition.trim();
    if let Some((from, to)) = trimmed.split_once("->") {
        format!(
            "{}->{}",
            super::compat::v1::normalize_state_name(from.trim()),
            super::compat::v1::normalize_state_name(to.trim()),
        )
    } else {
        trimmed.to_string()
    }
}

/// Render the policy in human-readable format for `policy show`.
///
/// Only shows sections that are actually configured — no synthesized defaults.
pub fn render_policy_show(policy: &Policy) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Guards
    if !policy.guards.is_empty() {
        lines.push("guards:".into());
        for guard in &policy.guards {
            let rules: Vec<String> = guard.requires.iter().map(|r| r.to_string()).collect();
            lines.push(format!("  {}: {}", guard.on, rules.join(", ")));
        }
        lines.push(String::new());
    }

    // Checks
    lines.push("checks:".into());
    lines.push(format!("  strict = {}", policy.checks.strict));
    lines.push(String::new());

    // Creation rules (lifecycle-keyed, plus tag overlays).
    if !policy.creation_rules.is_empty() {
        lines.push("creation_rules:".into());
        let mut keys: Vec<&String> = policy.creation_rules.keys().collect();
        keys.sort();
        for key in keys {
            let entry = &policy.creation_rules[key];
            lines.push(format!("  {key}: {}", format_creation_rules(&entry.base)));
            let mut tags: Vec<&String> = entry.tag.keys().collect();
            tags.sort();
            for tag in tags {
                let rules = &entry.tag[tag];
                lines.push(format!("    tag.{tag}: {}", format_creation_rules(rules)));
            }
        }
        lines.push(String::new());
    }

    // Node rules
    if !policy.node_rules.is_empty() {
        lines.push("node_rules:".into());
        let mut keys: Vec<&String> = policy.node_rules.keys().collect();
        keys.sort();
        for key in keys {
            let types: Vec<String> = policy.node_rules[key]
                .iter()
                .map(|n| n.to_string())
                .collect();
            if types.is_empty() {
                lines.push(format!("  {key}: (none allowed)"));
            } else {
                lines.push(format!("  {key}: {}", types.join(", ")));
            }
        }
        lines.push(String::new());
    } else {
        lines.push("node_rules: (not configured)".into());
        lines.push(String::new());
    }

    // Revise rules
    if let Some(revise) = &policy.revise_rules {
        lines.push("revise_rules:".into());
        if !revise.allow_body_revise.is_empty() {
            lines.push(format!("  body: [{}]", revise.allow_body_revise.join(", ")));
        }
        if !revise.allow_node_revise.is_empty() {
            lines.push(format!("  node: [{}]", revise.allow_node_revise.join(", ")));
        }
        lines.push(String::new());
    } else {
        lines.push("revise_rules: (not configured)".into());
        lines.push(String::new());
    }

    // Evidence rules
    if let Some(evidence) = &policy.evidence_rules {
        lines.push("evidence_rules:".into());
        lines.push(format!("  allow: [{}]", evidence.allow_evidence.join(", ")));
        lines.push(String::new());
    } else {
        lines.push("evidence_rules: (not configured)".into());
        lines.push(String::new());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::event::ThreadKind;
    use super::*;

    fn make_policy(guards: Vec<GuardEntry>) -> Policy {
        let mut p = Policy {
            guards,
            ..Default::default()
        };
        let emitter = lint_emit::LintEmitter::new_capturing(None);
        super::super::compat::v1::rewrite_legacy_policy(
            &mut p,
            &emitter,
            std::path::Path::new("policy.toml"),
        );
        p
    }

    fn state_for(kind: ThreadKind) -> ThreadState {
        // Phase 2c: keep `lifecycle` consistent with `kind` so policy
        // predicates that key on lifecycle (lifecycle=proposal, etc.)
        // see the value the kind would have implied pre-Phase-2c.
        ThreadState {
            kind,
            lifecycle: kind.lifecycle(),
            ..Default::default()
        }
    }

    fn minimal_policy() -> Policy {
        make_policy(vec![GuardEntry {
            on: "under-review->accepted".into(),
            requires: vec![
                GuardRule::NoOpenObjections,
                GuardRule::NoOpenActions,
                GuardRule::OneHumanApproval,
            ],
            ..Default::default()
        }])
    }

    #[test]
    fn guards_for_matches_transition() {
        let policy = minimal_policy();
        assert_eq!(
            policy
                .guards_for("under-review->accepted", &state_for(ThreadKind::Rfc))
                .len(),
            1
        );
        assert!(policy
            .guards_for("draft->under-review", &state_for(ThreadKind::Rfc))
            .is_empty());
    }

    #[test]
    fn at_least_one_summary_scanner_names_lines() {
        // ADR-006: every non-comment line that mentions the removed
        // predicate produces a hit with the 1-based line number.
        let toml = "[[guards]]\non = \"x->y\"\nrequires = [\"at_least_one_summary\"]\n";
        let hits = scan_at_least_one_summary_warnings(toml);
        assert_eq!(hits, vec![AtLeastOneSummaryHit { line: 3 }]);
        assert!(AT_LEAST_ONE_SUMMARY_BODY.contains("at_least_one_summary"));
        assert!(AT_LEAST_ONE_SUMMARY_BODY.contains("removed"));
    }

    #[test]
    fn at_least_one_summary_scanner_ignores_comment_lines() {
        // A deprecation note in a TOML comment is not the predicate
        // firing — users should be able to leave such notes without
        // tripping the scanner.
        let toml = "# Per ADR-006, at_least_one_summary is removed.\n\
                    [[guards]]\non = \"x->y\"\nrequires = [\"no_open_objections\"]\n";
        let hits = scan_at_least_one_summary_warnings(toml);
        assert!(hits.is_empty(), "expected no hits; got: {hits:?}");
    }

    /// #6k7hq482: `Policy::load_with_emitter` must render the policy
    /// path repo-relative when the emitter knows the repo root, so the
    /// host-absolute path doesn't leak into stderr / forum events.
    #[test]
    fn load_emits_repo_relative_path_when_inside_repo() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join(".forum/policy.toml");
        std::fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
        std::fs::write(
            &policy_path,
            "[[guards]]\non = \"draft->open\"\nrequires = [\"at_least_one_summary\"]\n",
        )
        .unwrap();

        let emitter = LintEmitter::new_capturing(Some(dir.path().to_path_buf()));
        let _ = Policy::load_with_emitter(&policy_path, &emitter).unwrap();

        let captured = emitter.captured().unwrap();
        let summary_msg = captured
            .iter()
            .find(|m| m.contains("at_least_one_summary"))
            .expect("expected an at_least_one_summary warning");
        assert!(
            summary_msg.contains(".forum/policy.toml:3:"),
            "expected repo-relative prefix; got: {summary_msg}"
        );
        assert!(
            !summary_msg.contains(dir.path().to_str().unwrap()),
            "absolute repo path leaked into output: {summary_msg}"
        );
    }

    /// #6k7hq482: when the policy file lives outside the emitter's repo
    /// root, the warning falls back to the absolute path with an
    /// inline `(outside repo root)` note so users can see why.
    #[test]
    fn load_falls_back_to_absolute_path_when_outside_repo() {
        let policy_dir = tempfile::tempdir().unwrap();
        let other_repo = tempfile::tempdir().unwrap();
        let policy_path = policy_dir.path().join("policy.toml");
        std::fs::write(
            &policy_path,
            "[[guards]]\non = \"draft->open\"\nrequires = [\"at_least_one_summary\"]\n",
        )
        .unwrap();

        let emitter = LintEmitter::new_capturing(Some(other_repo.path().to_path_buf()));
        let _ = Policy::load_with_emitter(&policy_path, &emitter).unwrap();

        let captured = emitter.captured().unwrap();
        let summary_msg = captured
            .iter()
            .find(|m| m.contains("at_least_one_summary"))
            .expect("expected an at_least_one_summary warning");
        assert!(summary_msg.contains("outside repo root"));
        assert!(summary_msg.contains(policy_path.to_str().unwrap()));
    }

    /// #6k7hq482: a second `Policy::load_with_emitter` call sharing
    /// the same emitter must not re-emit warnings whose
    /// `(kind, path, line)` triple already fired.
    #[test]
    fn load_suppresses_repeat_warnings_in_same_emitter() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join(".forum/policy.toml");
        std::fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
        std::fs::write(
            &policy_path,
            "[[guards]]\non = \"draft->open\"\nrequires = [\"at_least_one_summary\"]\n",
        )
        .unwrap();

        let emitter = LintEmitter::new_capturing(Some(dir.path().to_path_buf()));
        let _ = Policy::load_with_emitter(&policy_path, &emitter).unwrap();
        let first = emitter.captured().unwrap().len();
        let _ = Policy::load_with_emitter(&policy_path, &emitter).unwrap();
        let second = emitter.captured().unwrap().len();

        assert!(first > 0, "first load should emit at least one warning");
        assert_eq!(
            first, second,
            "second load with same emitter should not add new warnings"
        );
    }

    /// #6k7hq482: `GIT_FORUM_LINT_VERBOSE=1` users want to see every
    /// warning every time. The emitter's verbose mode must override
    /// both layers of suppression.
    #[test]
    fn load_with_verbose_emitter_re_emits_every_call() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join(".forum/policy.toml");
        std::fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
        std::fs::write(
            &policy_path,
            "[[guards]]\non = \"draft->open\"\nrequires = [\"at_least_one_summary\"]\n",
        )
        .unwrap();

        let emitter = LintEmitter::new_capturing(Some(dir.path().to_path_buf())).with_verbose(true);
        let _ = Policy::load_with_emitter(&policy_path, &emitter).unwrap();
        let first = emitter.captured().unwrap().len();
        let _ = Policy::load_with_emitter(&policy_path, &emitter).unwrap();
        let second = emitter.captured().unwrap().len();

        assert!(first > 0);
        assert_eq!(
            second,
            first * 2,
            "verbose emitter should fire every call; got {first} then {second}"
        );
    }

    // @ltojzq9l: lint warns when an allow-list entry uses a 1.x name
    // without listing its 2.0 canonical sibling.
    #[test]
    fn lint_flags_legacy_only_state_names_in_allow_lists() {
        let policy = Policy {
            evidence_rules: Some(EvidenceRules {
                allow_evidence: vec!["under-review".into(), "reviewing".into()],
            }),
            revise_rules: Some(ReviseRules {
                allow_body_revise: vec!["pending".into()],
                allow_node_revise: vec!["closed".into()],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        let messages: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            messages
                .iter()
                .any(|m| m.contains("evidence_rules.allow_evidence") && m.contains("under-review")),
            "expected legacy-name warning for under-review; got: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("revise_rules.allow_body_revise") && m.contains("pending")),
            "expected legacy-name warning for pending; got: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("revise_rules.allow_node_revise") && m.contains("closed")),
            "expected legacy-name warning for closed; got: {messages:?}"
        );
    }

    // @ltojzq9l: a policy with both 1.x and 2.0 names listed during
    // migration should not warn — the canonical form is present, the
    // 1.x name is dual-tolerance scaffolding.
    #[test]
    fn lint_does_not_flag_legacy_names_when_canonical_is_also_listed() {
        let policy = Policy {
            evidence_rules: Some(EvidenceRules {
                allow_evidence: vec!["reviewing".into(), "review".into()],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        let legacy_warnings: Vec<&str> = diags
            .iter()
            .filter(|d| d.message.contains("legacy 1.x state name"))
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            legacy_warnings.is_empty(),
            "did not expect legacy-name warnings when canonical is listed; got: {legacy_warnings:?}"
        );
    }

    #[test]
    fn at_least_one_summary_stripped_from_loaded_guards() {
        let mut p = Policy {
            guards: vec![GuardEntry {
                on: "x->y".into(),
                requires: vec![
                    GuardRule::NoOpenObjections,
                    GuardRule::AtLeastOneSummary,
                    GuardRule::OneHumanApproval,
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let emitter = lint_emit::LintEmitter::new_capturing(None);
        super::super::compat::v1::rewrite_legacy_policy(
            &mut p,
            &emitter,
            std::path::Path::new("policy.toml"),
        );
        assert!(!p.guards[0]
            .requires
            .iter()
            .any(|r| matches!(r, GuardRule::AtLeastOneSummary)));
        assert_eq!(p.guards[0].requires.len(), 2);
    }

    #[test]
    fn at_least_one_summary_evaluate_is_noop() {
        let state = state_for(ThreadKind::Rfc);
        // Even if a programmatic caller smuggles the variant through,
        // evaluate_rule returns no violation (ADR-006 — predicate
        // semantically removed).
        assert!(evaluate_rule(&GuardRule::AtLeastOneSummary, &state).is_none());
    }

    #[test]
    fn lint_valid_policy_returns_empty_of_warnings() {
        // Post-2.0: legacy 1.x state names normalize to 2.0 names, so a
        // 1.x guard like `under-review->accepted` may now fire informational
        // multi-lifecycle Notes. Notes are advisory; the test checks no
        // *warnings* fire.
        let policy = minimal_policy();
        let warnings: Vec<_> = lint_policy(&policy)
            .into_iter()
            .filter(|d| d.level == LintLevel::Warn)
            .collect();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn lint_invalid_transition_reports_diag() {
        let policy = make_policy(vec![GuardEntry {
            on: "badvalue".into(),
            requires: vec![],
            ..Default::default()
        }]);
        assert!(!lint_policy(&policy).is_empty());
    }

    #[test]
    fn guard_rule_display() {
        assert_eq!(
            GuardRule::NoOpenObjections.to_string(),
            "no_open_objections"
        );
        assert_eq!(
            GuardRule::AtLeastOneSummary.to_string(),
            "at_least_one_summary"
        );
        assert_eq!(GuardRule::NoOpenActions.to_string(), "no_open_actions");
        assert_eq!(
            GuardRule::OneHumanApproval.to_string(),
            "one_human_approval"
        );
        assert_eq!(
            GuardRule::HasCommitEvidence.to_string(),
            "has_commit_evidence"
        );
    }

    #[test]
    fn lint_unknown_guard_from_state() {
        let policy = make_policy(vec![GuardEntry {
            on: "bogus->closed".into(),
            requires: vec![],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(diags
            .iter()
            .any(|d| d.level == LintLevel::Warn && d.message.contains("unknown state \"bogus\"")));
    }

    #[test]
    fn lint_unknown_guard_to_state() {
        let policy = make_policy(vec![GuardEntry {
            on: "open->fantasy".into(),
            requires: vec![],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("unknown state \"fantasy\"")));
    }

    #[test]
    fn lint_notes_multi_lifecycle_transition() {
        // Post-2.0: "open->closed" normalizes to "open->done", which is
        // reachable in proposal, execution, and record lifecycles.
        let policy = make_policy(vec![GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        let multi = diags
            .iter()
            .find(|d| d.message.contains("multiple lifecycles"));
        assert!(multi.is_some());
        assert_eq!(multi.unwrap().level, LintLevel::Note);
        assert!(multi.unwrap().message.contains("execution"));
        assert!(
            multi.unwrap().message.contains("`lifecycle=` predicate"),
            "should suggest a lifecycle= predicate"
        );
    }

    // Removed: `lint_no_note_for_kind_unique_transition`. Pre-2.0 the lint
    // could conclude that a transition like `under-review->accepted` was
    // RFC-only because it lived in only one of the four kind tables. With
    // the unified §3.1 graph and 1.x-name normalization, the transition
    // collapses to `review->done`, which is reachable in multiple
    // lifecycles, so the kind-uniqueness premise no longer holds. The
    // facet-scoped guidance from B4 replaces the multi-kind heuristic.

    #[test]
    fn lint_invalid_transition_for_any_kind() {
        // "draft->closed" is valid states but not a valid transition
        let policy = make_policy(vec![GuardEntry {
            on: "draft->closed".into(),
            requires: vec![],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(diags.iter().any(|d| d.level == LintLevel::Warn
            && d.message
                .contains("not a valid transition for any lifecycle")));
    }

    #[test]
    fn lint_unknown_state_in_revise_rules() {
        let policy = Policy {
            revise_rules: Some(ReviseRules {
                allow_body_revise: vec!["nonexistent".into()],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("revise_rules.allow_body_revise")
                && d.message.contains("unknown state \"nonexistent\"")));
    }

    #[test]
    fn lint_unknown_state_in_node_rules_key() {
        let mut node_rules = HashMap::new();
        node_rules.insert("imaginary".to_string(), vec![]);
        let policy = Policy {
            node_rules,
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags.iter().any(|d| d.message.contains("node_rules")
            && d.message.contains("unknown state \"imaginary\"")));
    }

    #[test]
    fn lint_unknown_state_in_evidence_rules() {
        let policy = Policy {
            evidence_rules: Some(EvidenceRules {
                allow_evidence: vec!["nope".into()],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags.iter().any(|d| d.message.contains("evidence_rules")
            && d.message.contains("unknown state \"nope\"")));
    }

    #[test]
    fn lint_default_policy_has_no_warnings() {
        let policy: Policy =
            toml::from_str(include_str!("../../tests/fixtures/policy_default.toml")).unwrap();
        let diags = lint_policy(&policy);
        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.level == LintLevel::Warn)
            .collect();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn render_policy_show_empty_policy() {
        let policy = Policy::default();
        let out = render_policy_show(&policy);
        assert!(out.contains("checks:"));
        assert!(out.contains("strict = false"));
        assert!(out.contains("node_rules: (not configured)"));
        assert!(out.contains("revise_rules: (not configured)"));
        assert!(out.contains("evidence_rules: (not configured)"));
    }

    #[test]
    fn render_policy_show_full_policy() {
        let policy: Policy =
            toml::from_str(include_str!("../../tests/fixtures/policy_default.toml")).unwrap();
        let out = render_policy_show(&policy);
        assert!(out.contains("guards:"));
        // 2.0 canonical guard transition (per @ltojzq9l).
        assert!(out.contains("review->done:"));
        assert!(out.contains("creation_rules:"));
        assert!(out.contains("checks:"));
    }

    // ---- allow-list gap detection (ISSUE-0095) ----

    #[test]
    fn lint_warns_when_kind_missing_from_allow_list() {
        // Only RFC states — Issue, Dec, Task kinds are completely absent
        let policy = Policy {
            revise_rules: Some(ReviseRules {
                allow_body_revise: vec!["draft".into(), "proposed".into()],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags.iter().any(|d| d.level == LintLevel::Warn
            && d.message.contains("allow_body_revise")
            && d.message.contains("no states for execution")));
        assert!(diags.iter().any(|d| d.level == LintLevel::Warn
            && d.message.contains("allow_body_revise")
            && d.message.contains("no states for execution")));
    }

    #[test]
    fn lint_no_gap_warning_when_kind_partially_covered() {
        // Has at least one non-terminal state per kind (even if not all)
        let policy = Policy {
            revise_rules: Some(ReviseRules {
                allow_body_revise: vec![
                    "open".into(),     // issue + task
                    "draft".into(),    // rfc
                    "proposed".into(), // rfc + dec
                ],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        let gap_warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.level == LintLevel::Warn && d.message.contains("no states for"))
            .collect();
        assert!(
            gap_warnings.is_empty(),
            "unexpected gap warnings: {gap_warnings:?}"
        );
    }

    #[test]
    fn lint_no_gap_warning_for_empty_allow_list() {
        // Empty list = not configured, not a gap
        let policy = Policy {
            revise_rules: Some(ReviseRules {
                allow_body_revise: vec![],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        let gap_warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("no states for"))
            .collect();
        assert!(
            gap_warnings.is_empty(),
            "empty list should not trigger gap warning"
        );
    }

    #[test]
    fn lint_gap_warning_includes_remediation_hint() {
        // Use `working` (Execution-only state, not in Proposal's allowed
        // set) so RFC's lifecycle has no covered non-terminal states.
        let policy = Policy {
            evidence_rules: Some(EvidenceRules {
                allow_evidence: vec!["working".into()],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        let rfc_gap = diags
            .iter()
            .find(|d| d.message.contains("no states for proposal"))
            .expect("should warn about missing RFC states");
        assert!(
            rfc_gap.message.contains("consider adding"),
            "should include remediation hint"
        );
        assert!(
            rfc_gap.message.contains("draft"),
            "should suggest RFC non-terminal states"
        );
    }

    // ---- legacy kind-scoped guard keys auto-translate to lifecycle ----

    #[test]
    fn legacy_kind_scoped_guard_matches_lifecycle_peers() {
        // SPEC-2.0 §7.1 / §10.4: `issue:` rewrites to `lifecycle=execution`,
        // which matches both Issue and Task threads (both are execution).
        // The `tag=bug` predicate is the canonical way to scope to bug-style
        // execution threads only.
        let policy = make_policy(vec![GuardEntry {
            on: "issue:open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        }]);
        assert_eq!(
            policy
                .guards_for("open->closed", &state_for(ThreadKind::Issue))
                .len(),
            1
        );
        assert_eq!(
            policy
                .guards_for("open->closed", &state_for(ThreadKind::Task))
                .len(),
            1
        );
        // Different lifecycle (proposal): not matched.
        assert!(policy
            .guards_for("open->closed", &state_for(ThreadKind::Rfc))
            .is_empty());
    }

    #[test]
    fn guards_for_unscoped_matches_all_kinds() {
        let policy = make_policy(vec![GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        }]);
        assert_eq!(
            policy
                .guards_for("open->closed", &state_for(ThreadKind::Issue))
                .len(),
            1
        );
        assert_eq!(
            policy
                .guards_for("open->closed", &state_for(ThreadKind::Task))
                .len(),
            1
        );
    }

    #[test]
    fn guards_for_union_of_scoped_and_unscoped() {
        let policy = make_policy(vec![
            GuardEntry {
                on: "open->closed".into(),
                requires: vec![GuardRule::NoOpenActions],
                ..Default::default()
            },
            GuardEntry {
                on: "lifecycle=execution AND tag=bug : open->closed".into(),
                requires: vec![GuardRule::HasCommitEvidence],
                ..Default::default()
            },
        ]);
        // Issue with `bug` tag: both guards apply.
        let mut bug_state = state_for(ThreadKind::Issue);
        bug_state.tags = vec!["bug".into()];
        assert_eq!(
            policy.guards_for("open->closed", &bug_state).len(),
            2,
            "tagged execution thread gets both guards"
        );
        // Issue without tag: only the unscoped guard applies.
        assert_eq!(
            policy
                .guards_for("open->closed", &state_for(ThreadKind::Issue))
                .len(),
            1
        );
    }

    #[test]
    fn lint_scoped_guard_valid() {
        let policy = make_policy(vec![GuardEntry {
            on: "dec:proposed->accepted".into(),
            requires: vec![GuardRule::NoOpenObjections],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.level == LintLevel::Warn)
            .collect();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn lint_scoped_guard_unknown_kind() {
        let policy = make_policy(vec![GuardEntry {
            on: "boguskind:open->closed".into(),
            requires: vec![],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(
            diags
                .iter()
                .any(|d| d.level == LintLevel::Warn && d.message.contains("unknown kind prefix")),
            "should warn about unknown kind prefix: {diags:?}"
        );
    }

    #[test]
    fn lint_scoped_guard_invalid_transition_for_lifecycle() {
        // DEC's lifecycle (record) does not allow `review`, so review->done
        // (or its 1.x equivalent under-review->accepted) is not reachable
        // for the record lifecycle. The post-rewrite warning names the
        // lifecycle (the predicate the guard now binds to), not the
        // legacy kind.
        let policy = make_policy(vec![GuardEntry {
            on: "dec:under-review->accepted".into(),
            requires: vec![],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(
            diags.iter().any(|d| d.level == LintLevel::Warn
                && d.message.contains("not a valid transition for record")),
            "should warn about invalid transition for record lifecycle: {diags:?}"
        );
    }

    #[test]
    fn lint_scoped_guard_skips_multi_kind_note() {
        // Scoped guard should not trigger multi-kind note
        let policy = make_policy(vec![GuardEntry {
            on: "issue:open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(
            !diags
                .iter()
                .any(|d| d.message.contains("multiple lifecycles")),
            "scoped guard should not trigger multi-kind note"
        );
    }

    #[test]
    fn parse_guard_on_unscoped() {
        let (predicate, transition, warning) = parse_guard_on("open->closed").unwrap();
        assert_eq!(predicate, FacetPredicate::Always);
        assert_eq!(transition, "open->closed");
        assert!(warning.is_none());
    }

    #[test]
    fn parse_guard_on_legacy_kind_rewrites_with_warning() {
        let (predicate, transition, warning) = parse_guard_on("dec:proposed->accepted").unwrap();
        assert_eq!(predicate, FacetPredicate::Lifecycle(Lifecycle::Record));
        assert_eq!(transition, "proposed->accepted");
        let w = warning.expect("legacy kind prefix should warn");
        assert!(w.contains("rewritten"));
        assert!(w.contains("lifecycle=record"));
    }

    #[test]
    fn parse_guard_on_unknown_kind_prefix_errors() {
        // Bare unknown prefix is rejected so lint can surface it.
        assert!(parse_guard_on("bogus:open->closed").is_err());
    }

    // ---- SPEC-2.0 §7.1 facet expression ----

    #[test]
    fn parse_facet_lifecycle_predicate() {
        let pred = parse_facet_predicate("lifecycle=proposal").unwrap();
        assert_eq!(pred, FacetPredicate::Lifecycle(Lifecycle::Proposal));
    }

    #[test]
    fn parse_facet_tag_predicate() {
        let pred = parse_facet_predicate("tag=cross-cutting").unwrap();
        assert_eq!(pred, FacetPredicate::Tag("cross-cutting".into()));
    }

    #[test]
    fn parse_facet_and_predicate() {
        let pred = parse_facet_predicate("lifecycle=proposal AND tag=cross-cutting").unwrap();
        let expected = FacetPredicate::And(
            Box::new(FacetPredicate::Lifecycle(Lifecycle::Proposal)),
            Box::new(FacetPredicate::Tag("cross-cutting".into())),
        );
        assert_eq!(pred, expected);
    }

    #[test]
    fn parse_facet_or_and_not_with_parens() {
        let pred =
            parse_facet_predicate("NOT tag=bug OR (lifecycle=record AND tag=archive)").unwrap();
        let expected = FacetPredicate::Or(
            Box::new(FacetPredicate::Not(Box::new(FacetPredicate::Tag(
                "bug".into(),
            )))),
            Box::new(FacetPredicate::And(
                Box::new(FacetPredicate::Lifecycle(Lifecycle::Record)),
                Box::new(FacetPredicate::Tag("archive".into())),
            )),
        );
        assert_eq!(pred, expected);
    }

    #[test]
    fn parse_facet_invalid_lifecycle_value() {
        assert!(parse_facet_predicate("lifecycle=bogus").is_err());
    }

    #[test]
    fn parse_facet_unknown_key() {
        assert!(parse_facet_predicate("kind=rfc").is_err());
    }

    #[test]
    fn facet_predicate_matches_threadstate() {
        let mut state = state_for(ThreadKind::Rfc);
        state.tags = vec!["cross-cutting".into()];
        let pred = parse_facet_predicate("lifecycle=proposal AND tag=cross-cutting").unwrap();
        assert!(pred.matches(&state));

        let mut other = state_for(ThreadKind::Issue);
        other.tags = vec!["cross-cutting".into()];
        assert!(
            !pred.matches(&other),
            "execution lifecycle should not match"
        );

        let mut tagless = state_for(ThreadKind::Rfc);
        tagless.tags = vec![];
        assert!(!pred.matches(&tagless), "missing tag should not match");
    }

    #[test]
    fn guards_for_facet_scoped_filters_by_predicate() {
        let policy = make_policy(vec![GuardEntry {
            on: "lifecycle=proposal AND tag=cross-cutting : review->done".into(),
            requires: vec![GuardRule::OneHumanApproval],
            ..Default::default()
        }]);

        let mut rfc_x = state_for(ThreadKind::Rfc);
        rfc_x.tags = vec!["cross-cutting".into()];
        assert_eq!(policy.guards_for("review->done", &rfc_x).len(), 1);

        let rfc_no_tag = state_for(ThreadKind::Rfc);
        assert!(policy.guards_for("review->done", &rfc_no_tag).is_empty());

        let mut task = state_for(ThreadKind::Task);
        task.tags = vec!["cross-cutting".into()];
        assert!(
            policy.guards_for("review->done", &task).is_empty(),
            "execution lifecycle should not match"
        );
    }
}
