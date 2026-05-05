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

// ---------------------------------------------------------------------
// ALLOW_LIST contents (notes kept outside the array literal — rustfmt
// re-indents trailing comments).
//
// Categories of entries below:
//
// 1. Structural / migration consumer — the legacy/* tree itself plus
//    `commands/migrate.rs` (the legitimate Phase 4 consumer of legacy
//    chains).
//
// 2. 3.0-native modules with v2-delegating impls (node, policy,
//    thread): Lifecycle/ThreadKind/ThreadStatus helpers still route
//    through legacy::workflow::SPEC and legacy::v1::normalize_state_name.
//    Steps 3g and 3h close these.
//
// 3. v2 read-path KEEP files (validate, commands/shared): residual v2
//    read-path code that hasn't been rewired yet. commands/shared.rs
//    uses `legacy::workflow::SPEC.preset_lookup` only inside
//    `parse_thread_kind` (the Ls/Shortlog `--kind` filter surface);
//    step 3f closes it.
//
// Already cleared by task 1v400j3l v3.1 follow-up:
//   - commands/state.rs (3a): shorthand resolution → policy::resolve_shorthand
//   - commands/thread_new.rs (3b): kind preset → policy::CategoryPreset
//   - commands/show.rs (3c): state-diagram → CategoryRegistry built_in
//   - commands/doctor.rs (3d): orphan-ref probe → 3.0-native tree-shape check
//   - commands/ls.rs (3e): test fixture rebuilt with 3.0-native imports
//   - commands/shortlog.rs (3e): terminal_state_date → snapshot::history walk
//
// Cleared earlier by Phase 4: the DELETE-list source files
// (state_change, write_ops, create, repair, repair_workflow, prune,
// purge, timeline, index, reindex, github, github_import, github_export,
// commands::repair_workflow) — entries gone with the files.
// evidence.rs cleared by Phase 4 Step 5.
// ---------------------------------------------------------------------
const ALLOW_LIST: &[&str] = &[
    "src/internal/legacy/mod.rs",
    "src/internal/legacy/v1.rs",
    "src/internal/legacy/event.rs",
    "src/internal/legacy/workflow.rs",
    "src/internal/node.rs",
    "src/internal/policy.rs",
    "src/internal/thread.rs",
    "src/internal/commands/migrate.rs",
    "src/internal/validate.rs",
    "src/internal/commands/shared.rs",
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
        "files outside ALLOW_LIST reach into `internal::legacy::*`:\n  {}\n\n\
         If a new module needs the v1 compat surface, justify and add to\n\
         ALLOW_LIST. The end state is no entries here outside legacy/ itself.",
        violations.join("\n  "),
    );
}

#[test]
fn allow_list_paths_exist() {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    for path in ALLOW_LIST {
        let full = PathBuf::from(crate_root).join(path);
        assert!(
            full.exists(),
            "ALLOW_LIST entry {path} does not exist on disk; remove it."
        );
    }
}

// ---------------------------------------------------------------------
// Permanent-exemption contract for v3.0.0
// ---------------------------------------------------------------------

/// The v3.0.0 baseline of permanent exemptions.
///
/// Per ADR-011 Decision 3, the original target was "only
/// `commands/migrate.rs` reaches `internal::legacy/*`". Phase 4
/// (task `913c4s9v`) shipped with a documented set of exemptions
/// instead; v3.1 task `1v400j3l` is closing them down. The remaining
/// entries are tracked categories of legacy access:
///
/// 1. Structural / migration consumer (legacy/* itself + migrate.rs).
/// 2. 3.0-native modules with v2-delegating impls (node, policy,
///    thread): Lifecycle/ThreadKind/ThreadStatus helpers still route
///    through legacy::workflow::SPEC and legacy::v1::normalize_state_name.
/// 3. v2 read-path KEEP files (validate, commands/shared): not yet
///    rewired off the legacy event-shape projection.
///
/// The remaining 4 entries close in v3.1 steps 3f-3i + 3j when the
/// SPEC-3.0 §3 Category surface fully replaces Lifecycle dispatch and
/// the v2 peer types (ThreadState.events, ThreadState.evidence_items,
/// the v2 NodeType enum, etc.) are removed.
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
    "src/internal/commands/shared.rs",
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
        "ALLOW_LIST must equal LEGACY_GATE_PERMANENT_EXEMPTIONS.\n\
         Extras (in ALLOW but not permanent): {:?}\n\
         Missing (permanent but not in ALLOW): {:?}\n\n\
         If a transitional KEEP file needs legacy access for a Phase 4\n\
         step, that file should be cleared (rewire) before merging — not\n\
         grandfathered through. The Lifecycle/ThreadKind/etc. delegations\n\
         are tracked for the v3.1 Category rewire (task 1v400j3l).",
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
    let file = syn::parse_file(synthetic).unwrap();
    let mut finder = LegacyImportFinder::default();
    finder.visit_file(&file);
    assert!(finder.found);
}

#[test]
fn detector_flags_grouped_use_through_legacy() {
    let synthetic = r#"
        use crate::internal::legacy::v1::{EventCodec, EventLog};
        pub fn forbidden() {}
    "#;
    let file = syn::parse_file(synthetic).unwrap();
    let mut finder = LegacyImportFinder::default();
    finder.visit_file(&file);
    assert!(finder.found);
}

#[test]
fn detector_flags_qualified_path_call_to_legacy() {
    let synthetic = r#"
        pub fn forbidden() {
            crate::internal::legacy::v1::EventCodec::default();
        }
    "#;
    let file = syn::parse_file(synthetic).unwrap();
    let mut finder = LegacyImportFinder::default();
    finder.visit_file(&file);
    assert!(finder.found);
}

#[test]
fn detector_ignores_legacy_subtype_field_and_local_names() {
    let synthetic = r#"
        pub struct Frame {
            pub legacy_subtype: Option<String>,
        }
        pub fn ok() {
            let legacy = 7;
            let _ = legacy;
        }
    "#;
    let file = syn::parse_file(synthetic).unwrap();
    let mut finder = LegacyImportFinder::default();
    finder.visit_file(&file);
    assert!(!finder.found);
}
