// Approval gate — coordinates between a waiting runtime task and the user's
// `decide_approval` command call.
//
// When the runtime encounters a `PolicyDecision::RequiresApproval` it:
//   1. Creates an `approval_requests` DB row.
//   2. Calls `ApprovalGate::register(approval_id)` to obtain a receiver.
//   3. Emits an event for the frontend.
//   4. Awaits the receiver (with a timeout).
//
// When the user approves/rejects via the Tauri command, `decide_approval`
// calls `ApprovalGate::resolve(approval_id, approved)`, which sends the
// boolean through the channel and wakes up the waiting runtime task.

use std::{
    collections::HashMap,
    sync::Mutex,
};

use tokio::sync::oneshot;
use tracing::warn;

// ---------------------------------------------------------------------------
// ApprovalGate
// ---------------------------------------------------------------------------

pub struct ApprovalGate {
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

impl ApprovalGate {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new pending approval.
    ///
    /// Returns a `oneshot::Receiver<bool>` that will receive `true` when
    /// approved or `false` when rejected.  The caller should await this
    /// receiver (ideally with a timeout) in the agent run task.
    pub fn register(&self, approval_id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(approval_id.to_string(), tx);
        rx
    }

    /// Resolve a pending approval with the user's decision.
    ///
    /// Returns `true` if the approval was found and the signal was sent,
    /// `false` if the approval was not found (already timed out, etc.).
    pub fn resolve(&self, approval_id: &str, approved: bool) -> bool {
        let sender = self.pending.lock().unwrap().remove(approval_id);
        match sender {
            Some(tx) => {
                if tx.send(approved).is_err() {
                    warn!(approval_id=%approval_id, "Approval receiver already dropped");
                }
                true
            }
            None => {
                warn!(approval_id=%approval_id, "Approval not found in gate (already resolved or timed out)");
                false
            }
        }
    }

    /// Remove a pending approval without sending a decision (e.g. on timeout).
    pub fn cancel(&self, approval_id: &str) {
        self.pending.lock().unwrap().remove(approval_id);
    }
}

impl Default for ApprovalGate {
    fn default() -> Self {
        Self::new()
    }
}
