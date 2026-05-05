//! Contamination audit gate per RFC 7ymtc4b2 Phase 4 (task 913c4s9v Step 0)
//! and ADR-011 Decision 3.
//!
//! Modules that disappear (DELETE) or relocate into `internal::legacy/`
//! (MOVE-TO-LEGACY) at Phase 4 completion are forbidden from being
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
//! passes day one. Every Phase 4 commit that rewires / relocates / deletes
//! a contaminated file removes its `ALLOW_LIST` entry in the same commit.
//! The final commit asserts `ALLOW_LIST` is empty (see
//! `final_audit_pass_allow_list_must_be_empty`, ignored until then).
//!
//! Rationale for two separate gates: legacy_gate enforces "post-Phase-2,
//! 2.0-native modules don't lean on legacy::v1 anymore"; contamination_audit
//! enforces "post-Phase-4, the entire event-chain runtime is gone from the
//! 3.0 binary except via the migrate command". Different invariants,
//! different lifecycles for their allow-lists.

use std::collections::BTreeSet;
use std::path::PathBuf;

use syn::visit::Visit;
use walkdir::WalkDir;

/// Modules whose existence in non-migrate code paths is what Phase 4
/// removes. The list combines the DELETE table (lines 152-168 of
/// `doc/internal/3.0-removal-plan.md`) with the MOVE-TO-LEGACY targets
/// `event` and `workflow` (lines 145-146).
const FORBIDDEN_MODULES: &[&str] = &[
    // MOVE-TO-LEGACY (relocate into internal::legacy/ during Phase 4 Step 2)
    "event",
    "workflow",
    // DELETE (removed wholesale during Phase 4 Step 3)
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

/// Permanent exemptions that survive Phase 4 completion.
///
/// `legacy/*` is structurally allowed — it is the relocation target.
/// `commands/migrate.rs` is the single sanctioned consumer of legacy
/// event-chain code post-Phase-4 (ADR-011 Decision 1). These three
/// stay in `ALLOW_LIST` forever; everything else must drop out as
/// Phase 4 commits land.
const PERMANENT_EXEMPTIONS: &[&str] = &[
    "src/internal/legacy/mod.rs",
    "src/internal/legacy/v1.rs",
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
///     any other file in the same set. Removed in Step 2 (move) or
///     Step 3 (delete).
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
    "src/internal/commands/migrate.rs",
    // (b) DELETE-list and MOVE-TO-LEGACY targets (removed in Steps 2-3).
    "src/internal/event.rs",
    "src/internal/workflow.rs",
    "src/internal/timeline.rs",
    "src/internal/index.rs",
    "src/internal/reindex.rs",
    "src/internal/create.rs",
    "src/internal/write_ops.rs",
    "src/internal/state_change.rs",
    "src/internal/repair.rs",
    "src/internal/repair_workflow.rs",
    "src/internal/prune.rs",
    "src/internal/purge.rs",
    // (note: src/internal/github.rs has no forbidden imports — only github_import/export
    // pull in event/state_change/etc. — so it's omitted here. It is still git rm'd in Step 3.)
    "src/internal/github_import.rs",
    "src/internal/github_export.rs",
    "src/internal/commands/repair_workflow.rs",
    // (c) KEEP files currently contaminated — Phase 4 Step 1 rewires.
    "src/internal/thread.rs",
    // node.rs cleared in Phase 4 Step 1f (RFC 7ymtc4b2, task 913c4s9v):
    // NodeType definition relocated into node.rs itself, so no
    // event:: import remains. Picks up legacy_gate exemption (the
    // canonical/legacy_subtype_label impls still call legacy::v1).
    "src/internal/evidence.rs",
    "src/internal/policy.rs",
    // operation_check.rs cleared in Phase 4 Step 1i (RFC 7ymtc4b2,
    // task 913c4s9v): normalize_state_name and NodeType both moved to
    // 3.0-native homes (policy.rs and node.rs) — no event:: import.
    "src/internal/validate.rs",
    // id_alloc.rs cleared in Phase 4 Step 1g (RFC 7ymtc4b2,
    // task 913c4s9v): switched event::ThreadKind to thread::ThreadKind.
    "src/internal/commands/show.rs",
    "src/internal/commands/ls.rs",
    // commands/diff.rs cleared in Phase 4 Step 1b (RFC 7ymtc4b2,
    // task 913c4s9v): revisions now derive from snapshot::history
    // commits whose tree changed body.md — no super::event imports.
    // commands/bulk.rs cleared in Phase 4 Step 1i: switched
    // event::normalize_state_name and event::ThreadKind to their
    // 3.0-native homes (policy.rs, thread.rs).
    "src/internal/commands/brief.rs",
    // commands/verify.rs cleared in Phase 4 Step 1i: switched
    // event::normalize_state_name and ThreadStatus / ThreadKind to
    // their 3.0-native homes; the residual `event::{self}` glob was
    // unused after the swap and dropped.
    "src/internal/commands/doctor.rs",
    "src/internal/commands/state.rs",
    "src/internal/commands/shortlog.rs",
    // commands/shorthand_say.rs cleared in Phase 4 Step 1f: switched
    // event::NodeType to node::NodeType; no other event:: import remains.
    "src/internal/commands/thread_new.rs",
    // commands/node.rs cleared in Phase 4 Step 1f: switched
    // event::NodeType to node::NodeType; no other event:: import remains.
    "src/internal/commands/shared.rs",
    "src/internal/tui/mod.rs",
    "src/internal/tui/state.rs",
    // tui/input.rs cleared in Phase 4 Step 1c (RFC 7ymtc4b2,
    // task 913c4s9v): index/reindex imports replaced by
    // snapshot::list walker.
    // tui/render.rs cleared in Phase 4 Step 1g: switched
    // event::ThreadKind to thread::ThreadKind.
    // tui/persist.rs cleared in Phase 4 Step 1c: ThreadRow now
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
        "Modules outside the contamination ALLOW_LIST import forbidden Phase-4 modules:\n  - {}\n\n\
         If this is an intentional new dependency on a transitional module, add it to ALLOW_LIST\n\
         in tests/contamination_audit_test.rs and explain in the commit message — but note that\n\
         the ALLOW_LIST is supposed to shrink to empty across Phase 4, not grow.\n\
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

/// Once Phase 4 finishes, only `PERMANENT_EXEMPTIONS` remain in
/// `ALLOW_LIST`. This test stays `#[ignore]` until the final Step 6
/// commit unmarks it; flipping it from `ignore` to active is what
/// locks the gate closed.
#[test]
#[ignore = "Phase 4 not complete; ALLOW_LIST still grandfathers in-flight contamination"]
fn final_audit_pass_allow_list_must_be_empty_except_permanent_exemptions() {
    let extras: Vec<&&str> = ALLOW_LIST
        .iter()
        .filter(|p| !PERMANENT_EXEMPTIONS.contains(*p))
        .collect();
    assert!(
        extras.is_empty(),
        "Phase 4 final pass: ALLOW_LIST must contain only the permanent exemptions \
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
