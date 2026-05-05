//! SPEC-2.0 §3.1 / §9.1 — single source of truth for 2.0 workflow
//! metadata.
//!
//! Before #34ith16h, lifecycle/state metadata lived across five
//! data-holding sites (`Lifecycle::allowed_states`, `UNIFIED_TRANSITIONS`,
//! `normalize_state_name` / `migrate_legacy_state` alias maps,
//! `KIND_PRESETS` and `shorthand_target_for_lifecycle` in `main.rs`)
//! plus several read-side consumers that embedded their own copy of the
//! same knowledge. Adding a new lifecycle required editing every site,
//! and any consumer that failed to route through them silently
//! diverged.
//!
//! [`WorkflowSpec`] is the single home for the 2.0-native data: per-
//! lifecycle allowed states, the unified transition graph, the kind
//! preset registry, and shorthand resolution. Adding a new lifecycle
//! requires editing the [`Lifecycle`] enum (one variant + the four
//! trivial conversion arms — `Display`, `as_str`, `parse`, `Default`
//! / `match`-exhaustiveness) and one row in the relevant data table
//! here.
//!
//! 1.x → 2.0 compatibility rules (state-name aliases,
//! `migrate_legacy_state`, `kind:`-prefixed guard scopes, etc.) live
//! one level over in [`super::v1`] per RFC 915yuegd P1.
//! `WorkflowSpec` calls into legacy to keep its lifecycle-aware query
//! methods forgiving of legacy state names while the alias data
//! itself stays out of the 2.0 source of truth.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::event::ThreadKind;

// --------------------------------------------------------------------
// `Lifecycle` (3-variant v2 enum). Relocated here from
// `internal::policy` in v3.1 step 3m (task `1v400j3l`). The enum is
// a v2 dispatch axis (proposal/execution/record); the SPEC-3.0
// successor is the snapshot's `category` string. Read paths derive
// the user-facing "lifecycle" label from category+tags via
// `policy::lifecycle_label_for`. The typed enum survives only inside
// `internal::legacy` where the v2 event-chain transition graph
// genuinely needs it.
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
        match self {
            Self::Proposal => "draft",
            Self::Execution | Self::Record => "open",
        }
    }

    /// SPEC-2.0 §3.1.1 — states reachable for this lifecycle.
    pub fn allowed_states(self) -> &'static [&'static str] {
        match self {
            Self::Proposal => &[
                "draft",
                "open",
                "review",
                "done",
                "rejected",
                "withdrawn",
                "deprecated",
            ],
            Self::Execution => &[
                "open",
                "working",
                "review",
                "done",
                "rejected",
                "deprecated",
            ],
            Self::Record => &["open", "done", "rejected", "deprecated"],
        }
    }

    pub fn allows_state(self, state: &str) -> bool {
        self.allowed_states().contains(&state)
    }
}

impl std::fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Per-lifecycle workflow data, owned in one place per SPEC-2.0 §3.1.
struct LifecycleData {
    lifecycle: Lifecycle,
    initial_state: &'static str,
    allowed_states: &'static [&'static str],
}

/// SPEC-2.0 §3.1 — single unified transition graph.
///
/// Every edge any lifecycle might need; per-lifecycle reachability is
/// derived by intersecting against the lifecycle's `allowed_states`.
const UNIFIED_TRANSITIONS: &[(&str, &str)] = &[
    ("draft", "open"),
    ("draft", "withdrawn"),
    ("open", "working"),
    ("open", "review"),
    ("open", "done"),
    ("open", "rejected"),
    ("open", "withdrawn"),
    ("working", "review"),
    ("working", "done"),
    ("working", "rejected"),
    ("review", "done"),
    ("review", "working"),
    ("review", "rejected"),
    ("done", "open"),
    ("rejected", "open"),
    ("done", "deprecated"),
    ("rejected", "deprecated"),
];

/// SPEC-2.0 §3.1.1 — per-lifecycle initial state and allowed states.
const LIFECYCLES: &[LifecycleData] = &[
    LifecycleData {
        lifecycle: Lifecycle::Proposal,
        initial_state: "draft",
        allowed_states: &[
            "draft",
            "open",
            "review",
            "done",
            "rejected",
            "withdrawn",
            "deprecated",
        ],
    },
    LifecycleData {
        lifecycle: Lifecycle::Execution,
        initial_state: "open",
        allowed_states: &[
            "open",
            "working",
            "review",
            "done",
            "rejected",
            "deprecated",
        ],
    },
    LifecycleData {
        lifecycle: Lifecycle::Record,
        initial_state: "open",
        allowed_states: &["open", "done", "rejected", "deprecated"],
    },
];

/// SPEC-2.0 §9.1 — kind preset registry (`git forum new <preset>`).
///
/// Each preset binds a user-facing name (and optional aliases) to a
/// (storage `kind`, canonical `lifecycle`, default tag set) tuple.
/// Aliases include legacy 1.x names (`ask`, `bug`, `job`); resolution
/// scans `name` then `aliases` and returns the first match.
pub struct KindPreset {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub kind: ThreadKind,
    pub lifecycle: Lifecycle,
    pub tags: &'static [&'static str],
}

const KIND_PRESETS: &[KindPreset] = &[
    KindPreset {
        name: "rfc",
        aliases: &[],
        kind: ThreadKind::Rfc,
        lifecycle: Lifecycle::Proposal,
        tags: &["cross-cutting"],
    },
    KindPreset {
        name: "dec",
        aliases: &[],
        kind: ThreadKind::Dec,
        lifecycle: Lifecycle::Record,
        tags: &[],
    },
    KindPreset {
        name: "task",
        aliases: &["job"],
        kind: ThreadKind::Task,
        lifecycle: Lifecycle::Execution,
        tags: &["task"],
    },
    KindPreset {
        name: "issue",
        aliases: &["ask", "bug"],
        kind: ThreadKind::Issue,
        lifecycle: Lifecycle::Execution,
        tags: &["bug"],
    },
];

/// State-change shorthand → concrete target state, keyed on lifecycle.
///
/// SPEC-2.0 §9.3 maps the everyday CLI verbs (`close`, `accept`,
/// `propose`, `pend`, `reject`, `withdraw`, `deprecate`, `reopen`) to a
/// concrete 2.0 state in the thread's lifecycle, or to a typed rejection
/// hint when the verb does not apply to that lifecycle.
pub enum ShorthandResolution {
    /// Resolved to a concrete 2.0 state name.
    Target(&'static str),
    /// Verb does not apply to this lifecycle. Carries the operator-facing
    /// hint (e.g. `"close is rejected on a proposal thread — use \`accept\`"`).
    NotApplicable(&'static str),
    /// Verb is not in the shorthand table at all.
    Unknown,
}

/// Zero-sized handle for the workflow data tables.
///
/// Use the [`SPEC`] constant to access methods, e.g. `SPEC.initial_state(lifecycle)`.
pub struct WorkflowSpec;

/// The single, process-wide [`WorkflowSpec`] handle.
pub const SPEC: WorkflowSpec = WorkflowSpec;

impl WorkflowSpec {
    /// SPEC-2.0 §3.1.1 — initial state for a fresh thread of `lifecycle`.
    pub fn initial_state(&self, lifecycle: Lifecycle) -> &'static str {
        lifecycle_data(lifecycle).initial_state
    }

    /// SPEC-2.0 §3.1.1 — states reachable for `lifecycle`.
    pub fn allowed_states(&self, lifecycle: Lifecycle) -> &'static [&'static str] {
        lifecycle_data(lifecycle).allowed_states
    }

    /// `true` iff `state` is in `lifecycle`'s allowed-state set.
    pub fn allows_state(&self, lifecycle: Lifecycle, state: &str) -> bool {
        self.allowed_states(lifecycle).contains(&state)
    }

    /// SPEC-2.0 §3.1 — full unified transition graph (every edge any
    /// lifecycle might need). Per-lifecycle reachability is filtered via
    /// [`Self::allowed_states`].
    pub fn unified_transitions(&self) -> &'static [(&'static str, &'static str)] {
        UNIFIED_TRANSITIONS
    }

    /// `true` iff `from -> to` is a valid edge for `lifecycle`.
    ///
    /// Inputs may be 1.x state names; the 1.x → 2.0 alias fold is done
    /// via [`super::v1::normalize_state_name`]. Both endpoints
    /// must be in the lifecycle's allowed set (§3.1.1) and the edge
    /// must exist in the unified §3.1 graph.
    pub fn is_valid_transition(&self, lifecycle: Lifecycle, from: &str, to: &str) -> bool {
        let from = super::v1::normalize_state_name(from);
        let to = super::v1::normalize_state_name(to);
        self.allows_state(lifecycle, from)
            && self.allows_state(lifecycle, to)
            && UNIFIED_TRANSITIONS
                .iter()
                .any(|&(s, d)| s == from && d == to)
    }

    /// Destination states reachable in one step from `from` for `lifecycle`.
    /// Returns 2.0 state names; `from` may be a 1.x name (folded via
    /// [`super::v1::normalize_state_name`]).
    pub fn valid_targets(&self, lifecycle: Lifecycle, from: &str) -> Vec<&'static str> {
        let from = super::v1::normalize_state_name(from);
        UNIFIED_TRANSITIONS
            .iter()
            .filter_map(|&(s, d)| (s == from && self.allows_state(lifecycle, d)).then_some(d))
            .collect()
    }

    /// Shortest path from `from` to `to` via BFS over the unified graph,
    /// restricted to states allowed for `lifecycle`. `from`/`to` may be
    /// 1.x names (folded via [`super::v1::normalize_state_name`]).
    pub fn find_path(
        &self,
        lifecycle: Lifecycle,
        from: &str,
        to: &str,
    ) -> Option<Vec<&'static str>> {
        let from = super::v1::normalize_state_name(from);
        let to = super::v1::normalize_state_name(to);
        if from == to {
            return Some(vec![]);
        }
        if !self.allows_state(lifecycle, to) {
            return None;
        }
        let mut queue: VecDeque<(&str, Vec<&'static str>)> = VecDeque::new();
        let mut visited: Vec<&str> = vec![from];

        for &(src, dst) in UNIFIED_TRANSITIONS {
            if src == from && self.allows_state(lifecycle, dst) {
                if dst == to {
                    return Some(vec![dst]);
                }
                visited.push(dst);
                queue.push_back((dst, vec![dst]));
            }
        }

        while let Some((current, path)) = queue.pop_front() {
            for &(src, dst) in UNIFIED_TRANSITIONS {
                if src == current && self.allows_state(lifecycle, dst) && !visited.contains(&dst) {
                    let mut new_path = path.clone();
                    new_path.push(dst);
                    if dst == to {
                        return Some(new_path);
                    }
                    visited.push(dst);
                    queue.push_back((dst, new_path));
                }
            }
        }
        None
    }

    /// SPEC-2.0 §2.3.3 / §9.1 — the canonical lifecycle facet for a 1.x
    /// `ThreadKind`. Sourced from the kind preset table: every primary
    /// preset declares a `(kind, lifecycle)` pair, and this method finds
    /// the preset whose `kind` matches.
    pub fn kind_lifecycle(&self, kind: ThreadKind) -> Lifecycle {
        KIND_PRESETS
            .iter()
            .find(|p| p.kind == kind)
            .expect("every ThreadKind variant has a primary preset row in KIND_PRESETS")
            .lifecycle
    }

    /// SPEC-2.0 §9.1 — full kind preset registry (used by the CLI to
    /// list valid `<preset>` names in error messages).
    pub fn presets(&self) -> &'static [KindPreset] {
        KIND_PRESETS
    }

    /// SPEC-2.0 §9.1 — look up a preset by name OR alias. Returns `None`
    /// if `name` is not the primary name or alias of any preset.
    pub fn preset_lookup(&self, name: &str) -> Option<&'static KindPreset> {
        KIND_PRESETS
            .iter()
            .find(|p| p.name == name || p.aliases.contains(&name))
    }

    /// SPEC-2.0 §9.3 — resolve a state-change shorthand verb to a target
    /// state, keyed on `lifecycle`. See [`ShorthandResolution`].
    ///
    /// Inputs are the canonical 2.0 verb spellings (`closed`, `accepted`,
    /// `proposed`, `pending`, `rejected`, `deprecated`, `withdrawn`,
    /// `open`); upstream callers normalize their input before reaching
    /// this method.
    pub fn shorthand_target(&self, shorthand: &str, lifecycle: Lifecycle) -> ShorthandResolution {
        use Lifecycle::*;
        use ShorthandResolution::*;
        match (shorthand, lifecycle) {
            ("closed", Execution | Record) => Target("done"),
            ("closed", Proposal) => {
                NotApplicable("close is rejected on a proposal thread — use `accept`")
            }

            ("accepted", Proposal | Record) => Target("done"),
            ("accepted", Execution) => {
                NotApplicable("accept is rejected on an execution thread — use `close`")
            }

            ("proposed", Proposal) => Target("open"),
            ("proposed", _) => NotApplicable("propose is only valid on proposal threads"),

            ("pending", Execution) => Target("working"),
            ("pending", _) => NotApplicable("pend is only valid on execution threads"),

            ("rejected", _) => Target("rejected"),
            ("deprecated", _) => Target("deprecated"),

            ("withdrawn", Proposal) => Target("withdrawn"),
            ("withdrawn", _) => NotApplicable(
                "withdraw is only valid on proposal threads — use `close` or `reject`",
            ),

            // Unified `open` (thread reopen) — keep a single edge for every
            // lifecycle and let the state machine reject unreachable cases.
            ("open", _) => Target("open"),
            _ => Unknown,
        }
    }
}

fn lifecycle_data(lifecycle: Lifecycle) -> &'static LifecycleData {
    LIFECYCLES
        .iter()
        .find(|d| d.lifecycle == lifecycle)
        .expect("every Lifecycle variant has a row in the LIFECYCLES table")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lifecycle_has_a_data_row() {
        // Exhaustiveness guard: matches the Lifecycle enum.
        for &lc in &[Lifecycle::Proposal, Lifecycle::Execution, Lifecycle::Record] {
            let _ = lifecycle_data(lc);
        }
    }

    #[test]
    fn every_thread_kind_has_a_primary_preset() {
        for &k in &[
            ThreadKind::Issue,
            ThreadKind::Rfc,
            ThreadKind::Dec,
            ThreadKind::Task,
        ] {
            let _ = SPEC.kind_lifecycle(k);
        }
    }

    #[test]
    fn preset_lookup_finds_aliases() {
        assert_eq!(SPEC.preset_lookup("issue").map(|p| p.name), Some("issue"));
        assert_eq!(SPEC.preset_lookup("ask").map(|p| p.name), Some("issue"));
        assert_eq!(SPEC.preset_lookup("bug").map(|p| p.name), Some("issue"));
        assert_eq!(SPEC.preset_lookup("job").map(|p| p.name), Some("task"));
        assert_eq!(SPEC.preset_lookup("rfc").map(|p| p.name), Some("rfc"));
        assert_eq!(SPEC.preset_lookup("dec").map(|p| p.name), Some("dec"));
        assert!(SPEC.preset_lookup("nope").is_none());
    }

    #[test]
    fn shorthand_close_rejected_on_proposal() {
        let r = SPEC.shorthand_target("closed", Lifecycle::Proposal);
        assert!(matches!(r, ShorthandResolution::NotApplicable(hint) if hint.contains("accept")));
    }

    #[test]
    fn shorthand_open_works_for_every_lifecycle() {
        for &lc in &[Lifecycle::Proposal, Lifecycle::Execution, Lifecycle::Record] {
            assert!(matches!(
                SPEC.shorthand_target("open", lc),
                ShorthandResolution::Target("open"),
            ));
        }
    }

    #[test]
    fn shorthand_unknown_verb() {
        assert!(matches!(
            SPEC.shorthand_target("nonsense", Lifecycle::Proposal),
            ShorthandResolution::Unknown,
        ));
    }
}
