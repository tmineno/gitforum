#![allow(dead_code)]

use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::evidence::EvidenceKind;

/// Defines an actor that participates in a scenario.
pub struct ActorDef {
    pub name: String,
    pub role: String,
    pub description: String,
}

/// Defines a thread to be created within a phase.
pub struct ThreadDef {
    pub kind: ThreadKind,
    pub title: String,
    pub body: String,
    pub creator: String,
    pub target_status: String,
}

/// Defines a discussion node to be added to a thread.
pub struct NodeDef {
    pub thread_ref: String,
    pub node_type: NodeType,
    pub body: String,
    pub actor: String,
    pub should_resolve: bool,
}

/// Defines a state transition for a thread.
pub struct StateTransitionDef {
    pub thread_ref: String,
    pub new_state: String,
    pub actor: String,
    pub sign_actors: Vec<String>,
}

/// Defines evidence to be attached to a thread.
pub struct EvidenceDef {
    pub thread_ref: String,
    pub kind: EvidenceKind,
    pub actor: String,
}

/// Defines a link between two threads.
pub struct LinkDef {
    pub from_thread_ref: String,
    pub to_thread_ref: String,
    pub rel: String,
    pub actor: String,
}

/// A phase groups threads, nodes, transitions, evidence, and links.
pub struct PhaseDef {
    pub name: String,
    pub threads: Vec<ThreadDef>,
    pub nodes: Vec<NodeDef>,
    pub transitions: Vec<StateTransitionDef>,
    pub evidence: Vec<EvidenceDef>,
    pub links: Vec<LinkDef>,
}

/// A complete scenario definition.
pub struct ScenarioDef {
    pub name: String,
    pub description: String,
    pub actors: Vec<ActorDef>,
    pub phases: Vec<PhaseDef>,
}

/// Expected outcome for a thread after scenario execution.
pub struct ExpectedOutcome {
    pub thread_ref: String,
    pub expected_status: String,
    pub min_nodes: usize,
    pub expected_evidence_count: usize,
    pub expected_link_count: usize,
}

/// Build the calculator project scenario used by both deterministic and live-agent tests.
pub fn calculator_scenario() -> ScenarioDef {
    ScenarioDef {
        name: "calculator".to_string(),
        description: "Multi-agent calculator project: RFCs, issues, evidence, contention"
            .to_string(),
        actors: vec![
            ActorDef {
                name: "human/alice".to_string(),
                role: "lead".to_string(),
                description: "Project lead. Creates RFCs, drives decisions, resolves objections."
                    .to_string(),
            },
            ActorDef {
                name: "human/bob".to_string(),
                role: "reviewer".to_string(),
                description: "Reviewer. Asks questions, raises objections, implements features."
                    .to_string(),
            },
            ActorDef {
                name: "ai/copilot".to_string(),
                role: "assistant".to_string(),
                description: "AI assistant. Identifies risks, proposes CLI designs.".to_string(),
            },
            ActorDef {
                name: "human/carol".to_string(),
                role: "developer".to_string(),
                description: "Developer. Implements bug fixes, attaches evidence.".to_string(),
            },
        ],
        phases: vec![
            // Phase 1: RFC Review
            PhaseDef {
                name: "rfc-review".to_string(),
                threads: vec![
                    ThreadDef {
                        kind: ThreadKind::Rfc,
                        title: "Calculator engine".to_string(),
                        body: "Core arithmetic engine for the calculator project.".to_string(),
                        creator: "human/alice".to_string(),
                        target_status: "accepted".to_string(),
                    },
                    ThreadDef {
                        kind: ThreadKind::Rfc,
                        title: "Input validation".to_string(),
                        body: "Validate user input at the CLI boundary.".to_string(),
                        creator: "human/bob".to_string(),
                        target_status: "rejected".to_string(),
                    },
                    ThreadDef {
                        kind: ThreadKind::Rfc,
                        title: "CLI interface".to_string(),
                        body: "User-facing CLI for the calculator.".to_string(),
                        creator: "ai/copilot".to_string(),
                        target_status: "draft".to_string(),
                    },
                ],
                nodes: vec![
                    // RFC-0001 nodes
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Question,
                        body: "What operations will be supported initially?".to_string(),
                        actor: "human/bob".to_string(),
                        should_resolve: false,
                    },
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Claim,
                        body: "We will support add, subtract, multiply, divide.".to_string(),
                        actor: "human/alice".to_string(),
                        should_resolve: false,
                    },
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Objection,
                        body: "Division by zero needs explicit handling.".to_string(),
                        actor: "human/bob".to_string(),
                        should_resolve: true,
                    },
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Risk,
                        body: "Floating-point precision may cause unexpected results.".to_string(),
                        actor: "ai/copilot".to_string(),
                        should_resolve: false,
                    },
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Action,
                        body: "Add div-by-zero guard to the engine spec.".to_string(),
                        actor: "human/alice".to_string(),
                        should_resolve: true,
                    },
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Summary,
                        body: "Consensus: 4 basic ops, div-by-zero returns error.".to_string(),
                        actor: "human/alice".to_string(),
                        should_resolve: false,
                    },
                    // Missing node types on RFC-0001
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Alternative,
                        body: "Consider a stack-based approach instead of direct evaluation."
                            .to_string(),
                        actor: "human/bob".to_string(),
                        should_resolve: false,
                    },
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Assumption,
                        body: "Assumes IEEE 754 double precision floating point.".to_string(),
                        actor: "ai/copilot".to_string(),
                        should_resolve: false,
                    },
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Review,
                        body: "Overall analysis: approach is sound, division handling addressed."
                            .to_string(),
                        actor: "human/carol".to_string(),
                        should_resolve: false,
                    },
                    NodeDef {
                        thread_ref: "RFC-0001".to_string(),
                        node_type: NodeType::Evidence,
                        body: "See benchmark results in bench/arithmetic.csv.".to_string(),
                        actor: "ai/copilot".to_string(),
                        should_resolve: false,
                    },
                    // RFC-0002 nodes
                    NodeDef {
                        thread_ref: "RFC-0002".to_string(),
                        node_type: NodeType::Objection,
                        body: "This duplicates existing validation. Not needed yet.".to_string(),
                        actor: "human/alice".to_string(),
                        should_resolve: false,
                    },
                ],
                transitions: vec![
                    // RFC-0001: draft -> proposed -> under-review -> accepted
                    StateTransitionDef {
                        thread_ref: "RFC-0001".to_string(),
                        new_state: "proposed".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "RFC-0001".to_string(),
                        new_state: "under-review".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "RFC-0001".to_string(),
                        new_state: "accepted".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec!["human/alice".to_string()],
                    },
                    // RFC-0002: draft -> rejected
                    StateTransitionDef {
                        thread_ref: "RFC-0002".to_string(),
                        new_state: "rejected".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                ],
                evidence: vec![],
                links: vec![],
            },
            // Phase 2: Implementation
            PhaseDef {
                name: "implementation".to_string(),
                threads: vec![
                    ThreadDef {
                        kind: ThreadKind::Issue,
                        title: "Implement add and subtract".to_string(),
                        body: "Implement the + and - operations.".to_string(),
                        creator: "human/alice".to_string(),
                        target_status: "closed".to_string(),
                    },
                    ThreadDef {
                        kind: ThreadKind::Issue,
                        title: "Implement multiply and divide".to_string(),
                        body: "Implement the * and / operations.".to_string(),
                        creator: "human/bob".to_string(),
                        target_status: "closed".to_string(),
                    },
                    ThreadDef {
                        kind: ThreadKind::Issue,
                        title: "Handle division by zero".to_string(),
                        body: "Return error instead of panicking on x/0.".to_string(),
                        creator: "human/carol".to_string(),
                        target_status: "closed".to_string(),
                    },
                    ThreadDef {
                        kind: ThreadKind::Issue,
                        title: "Contention test".to_string(),
                        body: "Used for concurrent write testing.".to_string(),
                        creator: "human/alice".to_string(),
                        target_status: "open".to_string(),
                    },
                ],
                nodes: vec![],
                transitions: vec![
                    StateTransitionDef {
                        thread_ref: "ISSUE-0001".to_string(),
                        new_state: "closed".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "ISSUE-0002".to_string(),
                        new_state: "closed".to_string(),
                        actor: "human/bob".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "ISSUE-0003".to_string(),
                        new_state: "closed".to_string(),
                        actor: "human/carol".to_string(),
                        sign_actors: vec![],
                    },
                ],
                evidence: vec![EvidenceDef {
                    thread_ref: "ISSUE-0003".to_string(),
                    kind: EvidenceKind::Commit,
                    actor: "human/carol".to_string(),
                }],
                links: vec![
                    LinkDef {
                        from_thread_ref: "ISSUE-0001".to_string(),
                        to_thread_ref: "RFC-0001".to_string(),
                        rel: "implements".to_string(),
                        actor: "human/alice".to_string(),
                    },
                    LinkDef {
                        from_thread_ref: "ISSUE-0002".to_string(),
                        to_thread_ref: "RFC-0001".to_string(),
                        rel: "implements".to_string(),
                        actor: "human/bob".to_string(),
                    },
                ],
            },
            // Phase 3: Expanded lifecycle (missing transitions)
            PhaseDef {
                name: "expanded-lifecycle".to_string(),
                threads: vec![
                    // RFC-0004: goes through under-review -> rejected -> deprecated
                    ThreadDef {
                        kind: ThreadKind::Rfc,
                        title: "Error reporting format".to_string(),
                        body: "Standardize error output format for the calculator.".to_string(),
                        creator: "human/bob".to_string(),
                        target_status: "deprecated".to_string(),
                    },
                    // ISSUE-0005: rejected then reopened
                    ThreadDef {
                        kind: ThreadKind::Issue,
                        title: "Add logging framework".to_string(),
                        body: "Add structured logging to the calculator.".to_string(),
                        creator: "human/carol".to_string(),
                        target_status: "open".to_string(),
                    },
                ],
                nodes: vec![],
                transitions: vec![
                    // RFC-0003: draft -> proposed -> draft (revert to draft)
                    StateTransitionDef {
                        thread_ref: "RFC-0003".to_string(),
                        new_state: "proposed".to_string(),
                        actor: "ai/copilot".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "RFC-0003".to_string(),
                        new_state: "draft".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                    // RFC-0004: draft -> proposed -> under-review -> rejected -> deprecated
                    StateTransitionDef {
                        thread_ref: "RFC-0004".to_string(),
                        new_state: "proposed".to_string(),
                        actor: "human/bob".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "RFC-0004".to_string(),
                        new_state: "under-review".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "RFC-0004".to_string(),
                        new_state: "rejected".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "RFC-0004".to_string(),
                        new_state: "deprecated".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                    // ISSUE-0005: open -> rejected -> open (reopen from rejected)
                    StateTransitionDef {
                        thread_ref: "ISSUE-0005".to_string(),
                        new_state: "rejected".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "ISSUE-0005".to_string(),
                        new_state: "open".to_string(),
                        actor: "human/carol".to_string(),
                        sign_actors: vec![],
                    },
                    // ISSUE-0001: closed -> open (reopen, then close again)
                    StateTransitionDef {
                        thread_ref: "ISSUE-0001".to_string(),
                        new_state: "open".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                    StateTransitionDef {
                        thread_ref: "ISSUE-0001".to_string(),
                        new_state: "closed".to_string(),
                        actor: "human/alice".to_string(),
                        sign_actors: vec![],
                    },
                ],
                evidence: vec![],
                links: vec![],
            },
            // Phase 4: Contention
            PhaseDef {
                name: "contention".to_string(),
                threads: vec![],
                nodes: vec![
                    NodeDef {
                        thread_ref: "ISSUE-0004".to_string(),
                        node_type: NodeType::Claim,
                        body: "Alice's concurrent note".to_string(),
                        actor: "human/alice".to_string(),
                        should_resolve: false,
                    },
                    NodeDef {
                        thread_ref: "ISSUE-0004".to_string(),
                        node_type: NodeType::Claim,
                        body: "Bob's concurrent note".to_string(),
                        actor: "human/bob".to_string(),
                        should_resolve: false,
                    },
                ],
                transitions: vec![],
                evidence: vec![],
                links: vec![],
            },
        ],
    }
}

/// Expected outcomes for the calculator scenario.
pub fn calculator_expected_outcomes() -> Vec<ExpectedOutcome> {
    vec![
        ExpectedOutcome {
            thread_ref: "RFC-0001".to_string(),
            expected_status: "accepted".to_string(),
            min_nodes: 10, // 6 original + 4 new (alternative, assumption, review, evidence)
            expected_evidence_count: 0,
            expected_link_count: 0,
        },
        ExpectedOutcome {
            thread_ref: "RFC-0002".to_string(),
            expected_status: "rejected".to_string(),
            min_nodes: 1,
            expected_evidence_count: 0,
            expected_link_count: 0,
        },
        ExpectedOutcome {
            thread_ref: "RFC-0003".to_string(),
            expected_status: "draft".to_string(),
            min_nodes: 0,
            expected_evidence_count: 0,
            expected_link_count: 0,
        },
        ExpectedOutcome {
            thread_ref: "ISSUE-0001".to_string(),
            expected_status: "closed".to_string(),
            min_nodes: 0,
            expected_evidence_count: 0,
            expected_link_count: 1,
        },
        ExpectedOutcome {
            thread_ref: "ISSUE-0002".to_string(),
            expected_status: "closed".to_string(),
            min_nodes: 0,
            expected_evidence_count: 0,
            expected_link_count: 1,
        },
        ExpectedOutcome {
            thread_ref: "ISSUE-0003".to_string(),
            expected_status: "closed".to_string(),
            min_nodes: 0,
            expected_evidence_count: 1,
            expected_link_count: 0,
        },
        ExpectedOutcome {
            thread_ref: "ISSUE-0004".to_string(),
            expected_status: "open".to_string(),
            min_nodes: 2,
            expected_evidence_count: 0,
            expected_link_count: 0,
        },
        ExpectedOutcome {
            thread_ref: "RFC-0004".to_string(),
            expected_status: "deprecated".to_string(),
            min_nodes: 0,
            expected_evidence_count: 0,
            expected_link_count: 0,
        },
        ExpectedOutcome {
            thread_ref: "ISSUE-0005".to_string(),
            expected_status: "open".to_string(),
            min_nodes: 0,
            expected_evidence_count: 0,
            expected_link_count: 0,
        },
    ]
}
