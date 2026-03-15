use chrono::{DateTime, Utc};

use super::event::NodeType;

/// A structured discussion node contributed via a `say` event.
///
/// Preconditions: constructed from a Say event during replay.
/// Postconditions: immutable after construction; state tracked via resolved/retracted flags.
/// Failure modes: none (plain data struct).
/// Side effects: none.
#[derive(Debug, Clone)]
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
}

impl Node {
    /// True when the node is neither resolved, retracted, nor incorporated.
    pub fn is_open(&self) -> bool {
        !self.resolved && !self.retracted && !self.incorporated
    }
}
