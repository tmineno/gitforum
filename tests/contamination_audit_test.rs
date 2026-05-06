//! Contamination audit gate per RFC `7ymtc4b2`, task `913c4s9v`,
//! and task `1v400j3l`.
//!
//! Modules that disappear (DELETE) or relocate into `internal::legacy/`
//! (MOVE-TO-LEGACY) at task `913c4s9v` completion are forbidden from being
//! imported outside `internal::legacy/` and `internal::commands::migrate`.
//! This is a stricter sibling of `tests/legacy_gate_test.rs`:
//!
//!   legacy_gate            — blocks `internal::legacy::*` imports outside the migrate adapter
//!   contamination_audit    — blocks `internal::{event,workflow,timeline,index,reindex,
//!                                                create,write_ops,state_change,repair,
//!                                                repair_workflow,prune,purge,
//!                                                github,github_import,github_export}`
//!                              outside legacy/ and commands/migrate
//!
//! `ALLOW_LIST` grandfathers the current contamination so the test
//! passes day one. Every task `913c4s9v` commit that rewires / relocates / deletes
//! a contaminated file removes its `ALLOW_LIST` entry in the same commit.
//! The final commit asserts `ALLOW_LIST` is empty (see
//! `final_audit_pass_allow_list_must_be_empty`, ignored until then).
//!
//! Rationale for two separate gates: legacy_gate enforces "task `1hg98odf`
//! left 2.0-native modules independent from legacy::v1"; contamination_audit
//! enforces "task `913c4s9v` removes the event-chain runtime from the
//! 3.0 binary except via the migrate command". Different invariants,
//! different lifecycles for their allow-lists.

use std::collections::BTreeSet;
use std::path::PathBuf;

use syn::visit::Visit;
use walkdir::WalkDir;

/// Modules whose existence in non-migrate code paths is what task `913c4s9v`
/// removes. The list combines the DELETE table (lines 152-168 of
/// `doc/internal/3.0-removal-plan.md`) with the MOVE-TO-LEGACY targets
/// `event` and `workflow` (lines 145-146).
const FORBIDDEN_MODULES: &[&str] = &[
    // MOVE-TO-LEGACY (relocate into internal::legacy/ during task `913c4s9v`)
    "event",
    "workflow",
    // DELETE (removed wholesale during task `913c4s9v`)
    "timeline",
    "index",
    "reindex",
    "create",
    "write_ops",
    "state_change",
    "repair",
    "repair_workflow",
    "prune",
    "purge",
    "github",
    "github_import",
    "github_export",
];

/// Permanent exemptions that survive task `913c4s9v` completion.
///
/// `legacy/*` is structurally allowed — it is the relocation target.
/// `commands/migrate.rs` is the single sanctioned consumer of legacy
/// event-chain code task `913c4s9v`. These three
/// stay in `ALLOW_LIST` forever; everything else must drop out as
/// task `913c4s9v` commits land.
const PERMANENT_EXEMPTIONS: &[&str] = &[
    "src/internal/legacy/mod.rs",
    "src/internal/legacy/v1.rs",
    // task `913c4s9v` — codex objection
    // 2ab3b2a4 issue 2: PERMANENT_EXEMPTIONS must include the relocated
    // event/workflow modules (now permanently inside legacy/) so the
    // `final_audit_pass_*` test passes once `#[ignore]` is dropped.
    // Both files structurally belong to the legacy/ tree.
    "src/internal/legacy/event.rs",
    "src/internal/legacy/workflow.rs",
    // task `1v400j3l`: event-chain replay relocated
    // here from `internal::thread`. Structurally inside legacy/.
    "src/internal/legacy/chain_replay.rs",
    "src/internal/commands/migrate.rs",
];

/// Files allowed to reference any of `FORBIDDEN_MODULES`.
///
/// Three categories make up the initial list:
///
/// (a) **Permanent exemptions** (see `PERMANENT_EXEMPTIONS`).
///
/// (b) **DELETE-list / MOVE-TO-LEGACY files themselves.** A file that
///     is going away (or moving into legacy/) is allowed to reference
///     any other file in the same set. Removal/relocation is tracked by
///     task `913c4s9v`.
///
/// (c) **Currently contaminated KEEP files.** These need real rewiring
///     before the file can drop out of ALLOW. Most of the contamination
///     is shared types (`event::NodeType`, `event::ThreadKind`,
///     `event::Lifecycle`, `event::ThreadStatus`,
///     `event::normalize_state_name`) that need to relocate to
///     3.0-native modules (`node.rs`, `thread.rs`, `policy.rs`); a
///     smaller set is true event-replay machinery
///     (`commands/show.rs::timeline`, `commands/diff.rs` over
///     `state.events`, `tui/*::index`/`reindex`).
///
/// Each ALLOW removal MUST land in the same commit as the rewire it
/// represents. The `allow_list_is_minimal` test below catches stale
/// (c)-category entries — a transitional file with zero forbidden
/// imports must drop out of `ALLOW_LIST`.
const ALLOW_LIST: &[&str] = &[
    // (a) Permanent exemptions.
    "src/internal/legacy/mod.rs",
    "src/internal/legacy/v1.rs",
    // task `913c4s9v`: event.rs and
    // workflow.rs relocated from `internal::` to `internal::legacy::`.
    // They sibling-import each other via `super::event` / `super::workflow`,
    // which the contamination detector flags as "anchored at internal".
    // Exempt them — they are structurally inside legacy/ and any
    // non-legacy importer is caught by `tests/legacy_gate_test.rs`.
    "src/internal/legacy/event.rs",
    "src/internal/legacy/workflow.rs",
    // task `1v400j3l`: event-chain replay machinery
    // relocated here from `internal::thread`. Sibling-imports
    // `super::event` and `super::workflow`; non-legacy importers
    // are caught by `tests/legacy_gate_test.rs`.
    "src/internal/legacy/chain_replay.rs",
    "src/internal/commands/migrate.rs",
    // (b) task `913c4s9v` deleted the
    //     entire DELETE-list (state_change, write_ops, create, repair,
    //     repair_workflow, prune, purge, timeline, index, reindex,
    //     github, github_import, github_export, commands::repair_workflow)
    //     so this category is now empty.
    // (c) task `913c4s9v` also cleared the last (c)-category entry
    //     (`tui/mod.rs`) by deleting its lone v2-fixture call site
    //     (`crate::internal::create::create_thread` in the
    //     `thread_detail_header_shows_lifecycle_tags_linked_panel`
    //     test, which exercised the v2 §2.3.3 kind→lifecycle/tags
    //     fallback removed by task `913c4s9v`).
    // tui/state.rs cleared in task `913c4s9v` (RFC 7ymtc4b2,
    // task `913c4s9v`): switched its lone `event::validate_tag` call to
    // `thread::validate_tag` (the helper relocated to thread.rs as a
    // SPEC-3.0 §2.3.5 grammar concern, not v2 event surface).
    // tui/input.rs cleared in task `913c4s9v` (RFC 7ymtc4b2,
    // task `913c4s9v`): index/reindex imports replaced by
    // snapshot::list walker.
    // tui/render.rs cleared in task `913c4s9v`: switched
    // event::ThreadKind to thread::ThreadKind.
    // tui/persist.rs cleared in task `913c4s9v`: ThreadRow now
    // imported from snapshot::list, no internal::index dependency.
];

/// Walks every `syn::Path` and `syn::UseTree` and records which forbidden
/// module names appear as a non-leaf segment.
///
/// A bare leaf identifier (variable name `event`, field `event_type`)
/// is not flagged — only multi-segment paths where a forbidden name
/// has another segment after it (`super::event::EventType`,
/// `crate::internal::index::ThreadRow`).
#[derive(Default)]
struct ContaminationFinder {
    forbidden: Vec<&'static str>,
    found: BTreeSet<String>,
}

impl ContaminationFinder {
    fn new(forbidden: &'static [&'static str]) -> Self {
        Self {
            forbidden: forbidden.to_vec(),
            found: BTreeSet::new(),
        }
    }

    fn matches_forbidden(&self, name: &str) -> Option<&'static str> {
        self.forbidden.iter().copied().find(|f| *f == name)
    }
}

/// A "reaches the internal tree" anchor before the forbidden name —
/// names like `event` are common in third-party crates (e.g.
/// `crossterm::event`), so the gate only fires when the path is
/// rooted at the project's own internal tree. Recognized anchors:
/// `crate::internal::<forbidden>`, `internal::<forbidden>`,
/// `super[::super]*::<forbidden>`.
fn segment_chain_targets_internal(segs: &[String], forbidden_idx: usize) -> bool {
    if forbidden_idx == 0 {
        return false;
    }
    let prefix = &segs[..forbidden_idx];
    // `super[::super]*::<forbidden>` — relative path from a sibling
    // module under `internal::*`.
    if prefix.iter().all(|s| s == "super") {
        return true;
    }
    // `crate::internal::<forbidden>` or `internal::<forbidden>`.
    let last = prefix.last().map(|s| s.as_str());
    if last == Some("internal") {
        return true;
    }
    false
}

impl<'ast> Visit<'ast> for ContaminationFinder {
    fn visit_path(&mut self, p: &'ast syn::Path) {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        for (i, name) in segs.iter().enumerate() {
            if i + 1 >= segs.len() {
                continue;
            }
            if let Some(hit) = self.matches_forbidden(name) {
                if segment_chain_targets_internal(&segs, i) {
                    self.found.insert(hit.to_string());
                }
            }
        }
        syn::visit::visit_path(self, p);
    }

    fn visit_use_tree(&mut self, t: &'ast syn::UseTree) {
        collect_use_tree_hits(t, &self.forbidden, &mut self.found, &[]);
        syn::visit::visit_use_tree(self, t);
    }
}

fn collect_use_tree_hits(
    t: &syn::UseTree,
    forbidden: &[&'static str],
    out: &mut BTreeSet<String>,
    prefix: &[String],
) {
    match t {
        syn::UseTree::Path(p) => {
            let name = p.ident.to_string();
            if let Some(hit) = forbidden.iter().copied().find(|f| *f == name) {
                let segs: Vec<String> = prefix
                    .iter()
                    .cloned()
                    .chain(std::iter::once(name.clone()))
                    .collect();
                if segment_chain_targets_internal(&segs, segs.len() - 1) {
                    out.insert(hit.to_string());
                }
            }
            let mut next_prefix = prefix.to_vec();
            next_prefix.push(name);
            collect_use_tree_hits(&p.tree, forbidden, out, &next_prefix);
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                collect_use_tree_hits(item, forbidden, out, prefix);
            }
        }
        // `use super::index;` (leaf) brings the forbidden module name
        // into the file's scope, where unprefixed paths like
        // `index::open_db()` then resolve. Flag it the same way the
        // Path arm above flags `use super::index::open_db`.
        syn::UseTree::Name(name_leaf) => {
            let name = name_leaf.ident.to_string();
            if let Some(hit) = forbidden.iter().copied().find(|f| *f == name) {
                let segs: Vec<String> = prefix
                    .iter()
                    .cloned()
                    .chain(std::iter::once(name.clone()))
                    .collect();
                if segment_chain_targets_internal(&segs, segs.len() - 1) {
                    out.insert(hit.to_string());
                }
            }
        }
        syn::UseTree::Rename(rename) => {
            let name = rename.ident.to_string();
            if let Some(hit) = forbidden.iter().copied().find(|f| *f == name) {
                let segs: Vec<String> = prefix
                    .iter()
                    .cloned()
                    .chain(std::iter::once(name.clone()))
                    .collect();
                if segment_chain_targets_internal(&segs, segs.len() - 1) {
                    out.insert(hit.to_string());
                }
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

#[test]
fn no_forbidden_imports_outside_allow_list() {
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

        let mut finder = ContaminationFinder::new(FORBIDDEN_MODULES);
        finder.visit_file(&file);
        if !finder.found.is_empty() {
            let hits: Vec<String> = finder.found.into_iter().collect();
            violations.push(format!("{} -> {}", rel_str, hits.join(", ")));
        }
    }

    assert!(
        violations.is_empty(),
        "Modules outside the contamination ALLOW_LIST import forbidden task `913c4s9v` modules:\n  - {}\n\n\
         If this is an intentional new dependency on a transitional module, add it to ALLOW_LIST\n\
         in tests/contamination_audit_test.rs and explain in the commit message — but note that\n\
         the ALLOW_LIST is supposed to shrink to empty across task `913c4s9v`, not grow.\n\
         If this is a regression, replace the import with the snapshot-native equivalent\n\
         (NodeType / ThreadKind / ThreadStatus etc. relocate to node.rs / thread.rs;\n\
         event-replay loops replace with snapshot-commit walks per SPEC-3.0 §5.4).",
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
            "ALLOW_LIST entry no longer exists: {} — remove it from the list \
             (the rewire/move/delete that retired this file should drop its entry too)",
            path
        );
    }
}

#[test]
fn allow_list_has_no_duplicates() {
    let mut seen = BTreeSet::new();
    for path in ALLOW_LIST {
        assert!(seen.insert(*path), "duplicate ALLOW_LIST entry: {}", path);
    }
}

/// The task `913c4s9v` invariant: `ALLOW_LIST` contains only the
/// permanent exemptions. From task `913c4s9v` forward, adding any
/// new transitional ALLOW entry fails CI.
#[test]
fn final_audit_pass_allow_list_must_be_empty_except_permanent_exemptions() {
    let extras: Vec<&&str> = ALLOW_LIST
        .iter()
        .filter(|p| !PERMANENT_EXEMPTIONS.contains(*p))
        .collect();
    assert!(
        extras.is_empty(),
        "task `913c4s9v` final pass: ALLOW_LIST must contain only the permanent exemptions \
         (legacy/, commands/migrate.rs); still grandfathered: {:?}",
        extras
    );
}

/// Catches stale ALLOW entries: a transitional (non-permanent) file
/// that grandfathers no actual forbidden import is dead weight and
/// should be removed in the same commit as the rewire that cleared it.
#[test]
fn allow_list_is_minimal() {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    let mut stale: Vec<&str> = Vec::new();

    for &path in ALLOW_LIST {
        if PERMANENT_EXEMPTIONS.contains(&path) {
            continue;
        }
        let full = PathBuf::from(crate_root).join(path);
        let src = std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {}", path, e));
        let file = syn::parse_file(&src).unwrap_or_else(|e| panic!("parse {}: {}", path, e));
        let mut finder = ContaminationFinder::new(FORBIDDEN_MODULES);
        finder.visit_file(&file);
        if finder.found.is_empty() {
            stale.push(path);
        }
    }

    assert!(
        stale.is_empty(),
        "ALLOW_LIST contains transitional entries with no actual forbidden imports — \
         remove them:\n  - {}\n\n\
         (A clean file must drop out of ALLOW in the same commit as the rewire that \
         cleared it; carrying it forward defeats the shrinking-allow-list discipline.)",
        stale.join("\n  - ")
    );
}

#[test]
fn detector_flags_use_statement_importing_forbidden_module() {
    let synthetic = r#"
        use crate::internal::event::EventType;
        use crate::internal::index::ThreadRow;
        pub fn forbidden() {}
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = ContaminationFinder::new(FORBIDDEN_MODULES);
    finder.visit_file(&file);
    assert!(
        finder.found.contains("event"),
        "detector failed to flag `use crate::internal::event::EventType`"
    );
    assert!(
        finder.found.contains("index"),
        "detector failed to flag `use crate::internal::index::ThreadRow`"
    );
}

#[test]
fn detector_flags_qualified_path_call_to_forbidden_module() {
    let synthetic = r#"
        pub fn forbidden() {
            super::event::write_event(&git, &ev);
            crate::internal::reindex::rebuild();
        }
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = ContaminationFinder::new(FORBIDDEN_MODULES);
    finder.visit_file(&file);
    assert!(
        finder.found.contains("event"),
        "detector failed to flag inline `super::event::write_event` call"
    );
    assert!(
        finder.found.contains("reindex"),
        "detector failed to flag inline `crate::internal::reindex::rebuild` call"
    );
}

#[test]
fn detector_flags_grouped_use_through_forbidden_module() {
    let synthetic = r#"
        use crate::internal::event::{EventType, NodeType};
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = ContaminationFinder::new(FORBIDDEN_MODULES);
    finder.visit_file(&file);
    assert!(
        finder.found.contains("event"),
        "detector failed to flag grouped `use crate::internal::event::{{...}}`"
    );
}

#[test]
fn detector_flags_leaf_use_of_forbidden_module_name_when_anchored_at_internal() {
    // `use crate::internal::event;` (or `use super::index;`) brings
    // the forbidden module name into the file's scope; subsequent
    // unprefixed paths like `event::EventType` or `index::open_db()`
    // would resolve through it. The detector must flag the leaf
    // `use` even though no symbol is dereferenced *in the use itself*.
    let synthetic = r#"
        use crate::internal::event;
        use super::index;
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = ContaminationFinder::new(FORBIDDEN_MODULES);
    finder.visit_file(&file);
    assert!(
        finder.found.contains("event"),
        "detector failed to flag `use crate::internal::event` (leaf): {:?}",
        finder.found
    );
    assert!(
        finder.found.contains("index"),
        "detector failed to flag `use super::index` (leaf): {:?}",
        finder.found
    );
}

#[test]
fn detector_ignores_leaf_use_when_not_anchored_at_internal() {
    // `crossterm::event` is a third-party module that happens to share
    // a name with our forbidden list. The anchor check (parent segment
    // `internal` or all-`super`) keeps it from tripping the gate.
    let synthetic = r#"
        use crossterm::event;
        use std::sync::atomic::Ordering;
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = ContaminationFinder::new(FORBIDDEN_MODULES);
    finder.visit_file(&file);
    assert!(
        finder.found.is_empty(),
        "detector flagged a third-party `event` module: {:?}",
        finder.found
    );
}

#[test]
fn detector_ignores_local_names_and_field_names() {
    let synthetic = r#"
        pub struct Row { pub event_type: String, pub index: u32 }
        pub fn ok(event: &str, index: usize) -> usize { let _ = event; index }
    "#;
    let file = syn::parse_file(synthetic).expect("synthetic source must parse");
    let mut finder = ContaminationFinder::new(FORBIDDEN_MODULES);
    finder.visit_file(&file);
    assert!(
        finder.found.is_empty(),
        "detector raised a false positive on field/local names: {:?}",
        finder.found
    );
}
