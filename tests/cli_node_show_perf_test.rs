//! Bug `bvdk2w48` regression: `git forum node show <NODE_ID>` (no
//! `--in <thread>` hint) must resolve through the cheap node→thread
//! reverse index and replay only the owning thread, not every
//! thread in the repo.
//!
//! Two assertions:
//!
//! 1. **Correctness at scale**: with 50 threads × 3 nodes each, every
//!    node id is reachable via [`thread::find_node`] and resolves to
//!    the right owning thread.
//! 2. **Constant-factor latency**: end-to-end time stays within a
//!    small multiple of a single [`thread::replay_thread`] call. The
//!    legacy implementation replayed every thread twice, so the
//!    factor was ~2 × N; the index-driven path should be roughly
//!    1 × N (the cheap ls-tree sweep) + a single replay.

mod support;

use std::time::Instant;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::snapshot::{self, store::write_snapshot, NodeWithBody, ThreadDocument};
use git_forum::internal::thread::{self, NodeIdIndex, ThreadSnapshot};

const THREAD_COUNT: usize = 50;
const NODES_PER_THREAD: usize = 3;

fn setup() -> (support::repo::TestRepo, GitOps) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git)
}

fn make_thread(git: &GitOps, idx: usize) -> String {
    let id = format!("perf{idx:04x}");
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let doc = ThreadDocument::new(ThreadSnapshot {
        schema_version: ThreadSnapshot::SCHEMA_VERSION,
        id: id.clone(),
        title: format!("Test thread {idx}"),
        category: "rfc".into(),
        status: "draft".into(),
        tags: vec![],
        created_at: now,
        created_by: "human/alice".into(),
        updated_at: now,
        updated_by: "human/alice".into(),
        branch: None,
        supersedes: vec![],
        visibility: Default::default(),
    });
    write_snapshot(git, &id, &doc, "create perf thread").unwrap();
    id
}

fn append_node(git: &GitOps, thread_id: &str, body: &str, seq: usize) -> String {
    let mut doc = snapshot::read_snapshot(git, thread_id).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    // Salt the alloc input so two nodes appended in the same second
    // do not collide when we batch-create the fixture.
    let salt = format!("{body}-{seq}");
    let id = id_alloc::alloc_bare_thread_id("human/alice", &salt, &now.to_rfc3339());
    doc.nodes.push(NodeWithBody {
        record: NodeRecord {
            id: id.clone(),
            kind: NodeKind::Comment,
            status: NodeStatus::Open,
            created_at: now,
            created_by: "human/alice".into(),
            updated_at: None,
            updated_by: None,
            reply_to: None,
            legacy_label: None,
        },
        body: body.into(),
    });
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = "human/alice".into();
    write_snapshot(git, thread_id, &doc, "append perf node").unwrap();
    id
}

fn build_fixture(git: &GitOps) -> Vec<(String, Vec<String>)> {
    let mut threads_with_nodes = Vec::with_capacity(THREAD_COUNT);
    for t in 0..THREAD_COUNT {
        let thread_id = make_thread(git, t);
        let mut nodes = Vec::with_capacity(NODES_PER_THREAD);
        for n in 0..NODES_PER_THREAD {
            nodes.push(append_node(
                git,
                &thread_id,
                &format!("body t{t}-n{n}"),
                t * NODES_PER_THREAD + n,
            ));
        }
        threads_with_nodes.push((thread_id, nodes));
    }
    threads_with_nodes
}

#[test]
fn node_id_index_resolves_every_node_at_scale() {
    let (_repo, git) = setup();
    let threads_with_nodes = build_fixture(&git);

    let index = NodeIdIndex::build(&git).unwrap();

    for (thread_id, nodes) in &threads_with_nodes {
        for node_id in nodes {
            let (resolved, owning_thread) = index
                .resolve(node_id)
                .unwrap_or_else(|e| panic!("node {node_id} unresolved: {e}"));
            assert_eq!(&resolved, node_id);
            assert_eq!(
                &owning_thread, thread_id,
                "node {node_id} indexed to wrong thread"
            );
        }
    }

    // find_node should hand back a fully-populated NodeLookup whose
    // body text matches the snapshot we wrote — proving the lookup
    // does replay the owning thread, not just stop at the index.
    let (probe_thread, probe_nodes) = &threads_with_nodes[THREAD_COUNT / 2];
    let probe_node = &probe_nodes[1];
    let lookup = thread::find_node(&git, probe_node).unwrap();
    assert_eq!(&lookup.thread_id, probe_thread);
    assert_eq!(&lookup.node.record.id, probe_node);
    assert!(lookup.node.body.starts_with("body t"));
}

#[test]
fn node_show_stays_within_constant_factor_of_thread_show() {
    // The bug-report measurement showed legacy `node show` was ~115×
    // slower than `show @<thread>` because it replayed every thread
    // twice. Post-fix we expect the cost to be one cheap ls-tree
    // sweep plus one replay — i.e. on the same order as a single
    // replay_thread call, even with 50 threads.
    let (_repo, git) = setup();
    let threads_with_nodes = build_fixture(&git);

    let probe_thread = threads_with_nodes[0].0.clone();
    let probe_node = threads_with_nodes[THREAD_COUNT / 2].1[1].clone();

    let t0 = Instant::now();
    thread::replay_thread(&git, &probe_thread).unwrap();
    let single_replay = t0.elapsed();

    let t1 = Instant::now();
    thread::find_node(&git, &probe_node).unwrap();
    let node_show_cost = t1.elapsed();

    // Hard ceiling: the acceptance criterion calls for "well under
    // one second on a warm cache" at this scale. We give ourselves
    // 2s of headroom for slow CI runners; the real number on dev
    // hardware is well below 0.5s.
    assert!(
        node_show_cost.as_secs_f64() < 2.0,
        "find_node on {THREAD_COUNT} threads took {node_show_cost:?} (> 2s); \
         the bvdk2w48 reverse-index path appears to have regressed"
    );

    // Constant-factor bound vs a single replay. Pre-fix the ratio
    // was ~2 × THREAD_COUNT (≈100×); 15× absorbs noisy CI without
    // hiding a regression to the per-thread-replay path.
    let factor = node_show_cost.as_secs_f64() / single_replay.as_secs_f64().max(0.001);
    assert!(
        factor < 15.0,
        "node show / thread show factor = {factor:.1}× \
         ({node_show_cost:?} vs {single_replay:?}); \
         the bvdk2w48 reverse-index path appears to have regressed"
    );
}
