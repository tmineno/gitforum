use chrono::{DateTime, Utc};

use super::event::NodeType;

/// A structured discussion node contributed via a `say` event.
///
/// Preconditions: constructed from a Say event during replay.
/// Postconditions: immutable after construction; state tracked via resolved/retracted flags.
/// Failure modes: none (plain data struct).
/// Side effects: none.
///
/// `Default` is derived so test sites can elide unset optional fields with
/// struct-update syntax (e.g. `Node { node_id: …, node_type: …, body: …, ..Default::default() }`),
/// matching the pattern used on `Event`.
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub node_id: String,
    pub node_type: NodeType,
    pub body: String,
    pub actor: String,
    pub created_at: DateTime<Utc>,
    pub resolved: bool,
    pub retracted: bool,
    pub incorporated: bool,
    pub reply_to: Option<String>,
    /// SPEC-2.0 §2.5: rhetorical-subtype label preserved when the canonical
    /// `node_type` is `Comment` but the user (or migration tool) recorded a
    /// 1.x label like `claim` / `summary` / `risk` / `review` / `question` /
    /// `evidence` / `alternative` / `assumption`. `None` for canonical types
    /// or for native 2.0 `comment` writes.
    pub legacy_subtype: Option<String>,
}

impl Node {
    /// True when the node is neither resolved, retracted, nor incorporated.
    pub fn is_open(&self) -> bool {
        !self.resolved && !self.retracted && !self.incorporated
    }
}
