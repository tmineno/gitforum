//! Build-time gate per task `3dx6szoh` and ADR-011 Decision 2:
//! 3.0 modules must not import `internal::legacy::*`.
//!
//! Direction is asymmetric:
//!   legacy → 3.0  : OK (migration adapter writes 3.0 snapshots)
//!   3.0 → legacy  : FORBIDDEN
//!
//! Visibility (`pub(crate)`) is not sufficient — any sibling under
//! `src/internal/*` can reach a `pub(crate)` symbol. This test walks
//! `src/` with `syn`, finds every path that uses `legacy` as a non-leaf
//! module segment, and asserts the importer is on the allow-list.
//!
//! Allow-list (Phase 0 baseline) captures the current 2.0-native domain
//! modules that legitimately consume `internal::legacy::v1` per RFC
//! 915yuegd P1 (state-name folding, lifecycle auto-derive, kind-keyed
//! policy rewrites, NodeType canonical projection, legacy_subtype
//! preservation). The list shrinks as Phase 2 cutover commits move
//! domain code onto the snapshot path.

use std::path::PathBuf;

use syn::visit::Visit;
use walkdir::WalkDir;

/// Files allowed to reference `internal::legacy::*`.
///
/// Paths are relative to the crate root and use forward slashes.
/// When Phase 2 cuts a domain module off the legacy compat layer,
/// remove its entry — the test will then guard against regressions.
const ALLOW_LIST: &[&str] = &[
    // The legacy tree itself (internal references inside the module).
    "src/internal/legacy/mod.rs",
    "src/internal/legacy/v1.rs",
    // Phase 4 Step 2b (RFC 7ymtc4b2, task 913c4s9v): event.rs and
    // workflow.rs themselves now live under `legacy/`. Their internal
    // sibling-imports (legacy/event.rs uses super::workflow,
    // legacy/workflow.rs uses super::event) trigger the gate's path
    // walker; exempt them because they are inside the relocation
    // target.
    "src/internal/legacy/event.rs",
    "src/internal/legacy/workflow.rs",
    // 2.0-native domain code that delegates to legacy::v1 per RFC 915yuegd P1.
    // Phase 4 Step 1f (RFC 7ymtc4b2): NodeType relocated from event.rs
    // to node.rs; its `canonical` / `is_canonical` / `legacy_subtype_label`
    // impls still delegate to `legacy::v1` until Step 5 deletes the v2
    // NodeType + Node struct alongside the other v2 peer types.
    "src/internal/node.rs",
    "src/internal/policy.rs",
    "src/internal/thread.rs",
    // The migrate command — the legitimate Phase 4 consumer.
    "src/internal/commands/migrate.rs",
    // Phase 4 Step 2b (RFC 7ymtc4b2, task 913c4s9v): KEEP files with
    // residual v2 read-path code that was previously importing
    // `super::event::*` / `super::workflow::*` (then resolving via the
    // event.rs / workflow.rs ALLOW slots). Those files now reach into
    // `internal::legacy::event` / `internal::legacy::workflow` directly
    // and need their own exemption until Step 5 removes the v2 peer
    // types they're projecting.
    "src/internal/validate.rs",
    // evidence.rs cleared in Phase 4 Step 5 (RFC 7ymtc4b2,
    // task 913c4s9v): dropped Locator + add_evidence + add_thread_link
    // (all unreachable from runtime); the residual `Evidence` struct
    // doesn't import from legacy.
    "src/internal/commands/doctor.rs", // event::is_orphan_ref + legacy event chain probes
    // commands/state.rs cleared in task 1v400j3l step 3a: shorthand
    // resolution moved off legacy::workflow::SPEC onto the 3.0-native
    // `internal::policy::resolve_shorthand` (keyed on category + tags).
    "src/internal/commands/thread_new.rs", // workflow::{KindPreset, SPEC}
    "src/internal/commands/shared.rs",     // workflow::SPEC (test code)
    "src/internal/commands/show.rs",       // workflow::SPEC + test fixtures
    "src/internal/commands/ls.rs",         // event::* test fixtures
    "src/internal/commands/shortlog.rs",   // event::EventType test fixture
                                           // (Phase 4 Step 3 deleted the DELETE-list source files
                                           // (state_change, write_ops, create, repair, repair_workflow,
                                           // prune, purge, timeline, index, reindex, github, github_import,
                                           // github_export, commands::repair_workflow). Their entries are
                                           // gone with the files.)
];

/// Walks every `syn::Path` and records whether any of them uses
/// `legacy` as a non-leaf module segment (i.e. `legacy::*`).
///
/// A bare leaf identifier `legacy` (such as the field name
/// `legacy_subtype` or the local `let legacy = ...`) is not flagged —
/// only a multi-segment path with `legacy` followed by another segment.
#[derive(Default)]
struct LegacyImportFinder {
    found: bool,
}

impl<'ast> Visit<'ast> for LegacyImportFinder {
    fn visit_path(&mut self, p: &'ast syn::Path) {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        for (i, name) in segs.iter().enumerate() {
            if name == "legacy" && i + 1 < segs.len() {
                self.found = true;
                return;
            }
        }
        syn::visit::visit_path(self, p);
    }

    fn visit_use_tree(&mut self, t: &'ast syn::UseTree) {
        if use_tree_mentions_legacy(t) {
            self.found = true;
            return;
        }
        syn::visit::visit_use_tree(self, t);
    }
}

/// `use` paths are not represented as `syn::Path` — they are
/// `UseTree`s. Walk the tree explicitly looking for a `legacy`
/// segment that is not the leaf (i.e. `use ::legacy::v1::X` or
/// `use legacy::{a, b}` patterns).
fn use_tree_mentions_legacy(t: &syn::UseTree) -> bool {
    fn walk(t: &syn::UseTree, ancestors_have_legacy: bool) -> bool {
        match t {
            syn::UseTree::Path(p) => {
                let is_legacy = p.ident == "legacy";
                if is_legacy {
                    return true;
                }
                walk(&p.tree, ancestors_have_legacy)
            }
            syn::UseTree::Group(g) => {
                ancestors_have_legacy
                    || g.items.iter().any(|item| walk(item, ancestors_have_legacy))
            }
            // A leaf `legacy` identifier (`use foo::legacy;`) is allowed
            // because nothing is dereferenced through it.
            syn::UseTree::Name(_) | syn::UseTree::Rename(_) | syn::UseTree::Glob(_) => false,
        }
    }
    walk(t, false)
}

#[test]
fn no_legacy_imports_outside_allow_list() {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    let src_dir = PathBuf::from(crate_root).join("src");
    let mut violations: Vec<String> = Vec::new();

    for entry in WalkDir::new(&src_dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(crate_root)
            .expect("walked path is under crate root")
            .to_path_buf();
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if ALLOW_LIST.iter().any(|allowed| *allowed == rel_str) {
            continue;
        }

        let src = std::fs::read_to_string(entry.path())
            .unwrap_or_else(|e| panic!("read {}: {}", rel_str, e));
        let file = syn::parse_file(&src).unwrap_or_else(|e| panic!("parse {}: {}", rel_str, e));

        let mut finder = LegacyImportFinder::default();
        finder.visit_file(&file);
        if finder.found {
            violations.push(rel_str);
        }
    }

    assert!(
        violations.is_empty(),
        "Modules outside the allow-list import internal::legacy::*:\n  - {}\n\n\
         If this is a deliberate Phase 2/4 change, update ALLOW_LIST in tests/legacy_gate_test.rs.\n\
         If this is a regression, remove the import — 3.0 modules must reach legacy via\n\
         the snapshot/migration adapters only (ADR-011 Decision 2).",
        violations.join("\n  - ")
    );
}

#[test]
fn allow_list_paths_exist() {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    for path in ALLOW_LIST {
        let full = PathBuf::from(crate_root).join(path);
        assert!(
            full.exists(),
            "allow-list entry no longer exists: {} — remove it from ALLOW_LIST",
            path
        );
    }
}

/// Phase 4 Step 5 (RFC `7ymtc4b2`, task `913c4s9v`) — codex objection
/// 2ab3b2a4 issue 1: ADR-011 Decision 3 says only the migrate command
/// may consume legacy event-chain code. v3.0.0 cannot enforce that
/// strictly without the full Lifecycle→Category rewire (deferred from
/// Step 1j to v3.1 per body revision 4). The list below documents
/// every file v3.0.0 ships with as a sanctioned exemption — the test
/// directly below locks `ALLOW_LIST` to this exact set, so any new
/// transitional grandfathering trips CI.
///
/// Categories:
/// 1. **Structural** — inside `legacy/` itself: legacy/{mod, v1, event,
///    workflow}.rs.
/// 2. **Migration consumer** — commands/migrate.rs.
/// 3. **3.0-native modules with v2-delegating impls** — node.rs
///    (NodeType::canonical → legacy::v1), policy.rs (Lifecycle::*
///    methods → legacy::workflow::SPEC; normalize_state_name →
///    legacy::v1), thread.rs (replay_thread_at mixed-chain projection,
///    ThreadKind::lifecycle → legacy::workflow::SPEC).
/// 4. **v2 read-path KEEP files** — validate.rs (uses
///    legacy::event::EventType for shape checks); commands/state.rs,
///    thread_new.rs, shared.rs, show.rs (use legacy::workflow::SPEC
///    for state-diagram and category-keyed lookups);
///    commands/doctor.rs (uses legacy::event::is_orphan_ref for v2
///    ref triage); commands/ls.rs, shortlog.rs (test fixtures only).
///
/// Categories 3 and 4 close in v3.1 when the full SPEC-3.0 §3
/// Category surface replaces Lifecycle dispatch and the v2 peer types
/// (ThreadState.events, ThreadState.evidence_items, the v2 NodeType
/// enum, etc.) are removed. v3.0.0 ships with the exemption list as
/// the contract; the gate locks against drift below.
const LEGACY_GATE_PERMANENT_EXEMPTIONS: &[&str] = &[
    // Structural / migration consumer.
    "src/internal/legacy/mod.rs",
    "src/internal/legacy/v1.rs",
    "src/internal/legacy/event.rs",
    "src/internal/legacy/workflow.rs",
    "src/internal/commands/migrate.rs",
    // 3.0-native modules with v2-delegating impls.
    "src/internal/node.rs",
    "src/internal/policy.rs",
    "src/internal/thread.rs",
    // v2 read-path KEEP files (cleared in v3.1).
    "src/internal/validate.rs",
    "src/internal/commands/doctor.rs",
    "src/internal/commands/thread_new.rs",
    "src/internal/commands/shared.rs",
    "src/internal/commands/show.rs",
    "src/internal/commands/ls.rs",
    "src/internal/commands/shortlog.rs",
];

#[test]
fn allow_list_matches_permanent_set() {
    let extras: Vec<&&str> = ALLOW_LIST
        .iter()
        .filter(|p| !LEGACY_GATE_PERMANENT_EXEMPTIONS.contains(*p))
        .collect();
    let missing: Vec<&&str> = LEGACY_GATE_PERMANENT_EXEMPTIONS
        .iter()
        .filter(|p| !ALLOW_LIST.contains(*p))
        .collect();
    assert!(
        extras.is_empty() && missing.is_empty(),
        "v3.0.0 ALLOW_LIST must equal LEGACY_GATE_PERMANENT_EXEMPTIONS.\n\
         Extras (in ALLOW but not permanent): {:?}\n\
         Missing (permanent but not in ALLOW): {:?}\n\n\
         If a transitional KEEP file needs legacy access for a Phase 4\n\
         step, that file should be cleared (rewire) before merging — not\n\
         grandfathered through v3.0.0. The Lifecycle/ThreadKind/etc.\n\
         delegations are tracked for the v3.1 Category rewire.",
        extras,
        missing
    );
}

#[test]
fn detector_flags_use_statement_importing_legacy() {
    let synthetic = r#"
        use crate::internal::legacy::v1::EventCodec;
        pub fn forbidden() {}
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = LegacyImportFinder::default();
    finder.visit_file(&file);
    assert!(
        finder.found,
        "detector failed to flag `use crate::internal::legacy::v1::EventCodec`"
    );
}

#[test]
fn detector_flags_qualified_path_call_to_legacy() {
    let synthetic = r#"
        pub fn forbidden() {
            let _ = super::legacy::v1::normalize_state_name("open");
        }
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = LegacyImportFinder::default();
    finder.visit_file(&file);
    assert!(
        finder.found,
        "detector failed to flag inline `super::legacy::v1::normalize_state_name` call"
    );
}

#[test]
fn detector_flags_grouped_use_through_legacy() {
    let synthetic = r#"
        use crate::internal::legacy::{v1::A, v1::B};
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = LegacyImportFinder::default();
    finder.visit_file(&file);
    assert!(
        finder.found,
        "detector failed to flag `use crate::internal::legacy::{{...}}` group import"
    );
}

#[test]
fn detector_ignores_legacy_subtype_field_and_local_names() {
    let synthetic = r#"
        use crate::internal::policy::Policy;
        pub struct Event { pub legacy_subtype: Option<String> }
        pub fn ok(legacy_id: &str) -> &str { legacy_id }
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = LegacyImportFinder::default();
    finder.visit_file(&file);
    assert!(
        !finder.found,
        "detector raised a false positive on `legacy_subtype` / `legacy_id` identifiers"
    );
}
