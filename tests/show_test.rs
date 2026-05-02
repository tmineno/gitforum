//! Module integration tests for `src/internal/show.rs` rendering
//! (test-policy.md category 1). show-with-nodes, DEC/TASK kind
//! display, and Track G's `--tree` advisory tests land here in later
//! splits.

mod support;

use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::evidence;
use git_forum::internal::index;
use git_forum::internal::reindex;
use git_forum::internal::show;
use git_forum::internal::thread;
use git_forum::internal::write_ops;

use support::forum::{fixed_clock, make_dec, make_rfc, make_task, setup};

#[test]
fn show_contains_all_required_fields() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Initial thread body.\nSecond line."),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains(&id), "missing thread id");
    assert!(out.contains("Test RFC"), "missing title");
    // Phase 2b: lifecycle replaces kind on the show header.
    assert!(out.contains("proposal"), "missing lifecycle");
    assert!(out.contains("draft"), "missing status");
    assert!(out.contains("human/alice"), "missing actor");
    assert!(out.contains("---"), "missing body separator");
    assert!(out.contains("Initial thread body."), "missing body content");
    assert!(
        out.contains("Second line."),
        "missing multiline body content"
    );
    assert!(out.contains("2026-01-01T00:00:00Z"), "missing timestamp");
    assert!(out.contains("### timeline"), "missing timeline section");
    assert!(out.contains("date"), "missing timeline header");
    assert!(out.contains("node_id"), "missing node_id timeline header");
    assert!(out.contains("event_id"), "missing event_id timeline header");
    let event_id = &state.events[0].event_id;
    assert!(
        out.contains(&event_id[..event_id.len().min(16)]),
        "missing create event id in timeline"
    );
    assert!(
        out.contains("create"),
        "missing create event type in timeline"
    );
}

#[test]
fn show_replay_consistency() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state1 = thread::replay_thread(&git, &id).unwrap();
    let state2 = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(
        show::render_show(&state1, &show::ShowOptions::default()),
        show::render_show(&state2, &show::ShowOptions::default())
    );
}

#[test]
fn show_snapshot_contains_expected_fields() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Initial thread body."),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains(&id));
    assert!(out.contains("Test RFC"));
    // Phase 2b: lifecycle is the canonical axis on the show header.
    assert!(out.contains("proposal"));
    assert!(out.contains("draft"));
    assert!(out.contains("Initial thread body."));
    assert!(out.contains("human/alice"));
    assert!(out.contains("2026-01-01T00:00:00Z"));
    assert!(out.contains("create"));
}

// ---- show with nodes ----

#[test]
fn show_includes_open_objections_section() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Concern about performance.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains("**open objections:** 1"));
    assert!(out.contains("Concern about performance."));
}

#[test]
fn show_includes_latest_summary_section() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "This is the consensus.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains("latest summary:"));
    assert!(out.contains("This is the consensus."));
}

#[test]
fn show_no_extra_sections_when_no_nodes() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(!out.contains("open objections:"));
    assert!(!out.contains("open actions:"));
    assert!(!out.contains("latest summary:"));
}

#[test]
fn show_timeline_includes_say_events() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "This is important.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains(&node_id[..node_id.len().min(16)]));
    // SPEC-2.0 §2.5 / §9.3: legacy `claim` writes are canonicalized to
    // `comment`. Authors who want to preserve a rhetorical distinction
    // should encode it in the body (e.g. "Claim:" prefix).
    assert!(out.contains("comment"));
    assert!(out.contains("This is important."));
}

// ---- show --tree advisory ----

#[test]
fn show_tree_lists_only_implements_children_not_other_relations() {
    // SPEC-2.0 §2.1 / §B.4: `show --tree` lists only `--rel implements`
    // children, never other relations.
    let (_repo, git, paths) = setup();

    let parent = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Parent RFC",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let impl_child = create::create_thread(
        &git,
        ThreadKind::Task,
        "Implementing task",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let related_sibling = create::create_thread(
        &git,
        ThreadKind::Dec,
        "Related decision",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    evidence::add_thread_link(
        &git,
        &impl_child,
        &parent,
        "implements",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    evidence::add_thread_link(
        &git,
        &related_sibling,
        &parent,
        "relates-to",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();

    let conn = index::open_db(&db_path).unwrap();
    let rows = index::find_incoming_links(&conn, &parent, Some("implements")).unwrap();

    let parent_state = thread::replay_thread(&git, &parent).unwrap();
    let mut children = Vec::new();
    for row in &rows {
        let s = thread::replay_thread(&git, &row.from_thread_id).unwrap();
        children.push(show::TreeChild {
            id: s.id.clone(),
            title: s.title.clone(),
            lifecycle_label: s.lifecycle.as_str().to_string(),
            status: s.status.to_string(),
        });
    }

    let out = show::render_tree(&parent_state, &children);

    assert!(
        out.contains(&impl_child),
        "expected implements child in tree output:\n{out}"
    );
    assert!(out.contains("Implementing task"));
    assert!(
        !out.contains(&related_sibling),
        "relates-to sibling leaked into --tree output:\n{out}"
    );
    assert!(!out.contains("Related decision"));
}

// ---- DEC / TASK rendering ----

// Phase 2b: `kind` is no longer a primary display label; lifecycle
// (record / execution) is. The DEC and TASK kind labels still drive
// `git forum new <kind>` (per ADR-002), but `show` surfaces the
// lifecycle that kind maps to.

#[test]
fn show_dec_renders_record_lifecycle() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    let output = show::render_show(&state, &show::ShowOptions::default());
    assert!(output.contains("**lifecycle:** record"));
    assert!(output.contains("**status:**    open"));
}

#[test]
fn show_task_renders_execution_lifecycle() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    let output = show::render_show(&state, &show::ShowOptions::default());
    assert!(output.contains("**lifecycle:** execution"));
    assert!(output.contains("**status:**    open"));
}
