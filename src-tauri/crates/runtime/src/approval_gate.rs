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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_then_resolve_true_delivers_true() {
        let gate = ApprovalGate::new();
        let rx = gate.register("a1");

        assert!(gate.resolve("a1", true), "resolve should find the entry");
        assert!(rx.await.expect("sender kept alive"));
    }

    #[tokio::test]
    async fn register_then_resolve_false_delivers_false() {
        let gate = ApprovalGate::new();
        let rx = gate.register("a1");

        assert!(gate.resolve("a1", false));
        assert!(!rx.await.expect("sender kept alive"));
    }

    #[test]
    fn resolving_an_unknown_id_returns_false() {
        let gate = ApprovalGate::new();

        assert!(!gate.resolve("never-registered", true));
    }

    #[tokio::test]
    async fn a_second_resolve_of_the_same_id_returns_false() {
        // Guards against a double-click in the UI resolving twice.
        let gate = ApprovalGate::new();
        let rx = gate.register("a1");

        assert!(gate.resolve("a1", true));
        assert!(!gate.resolve("a1", false), "the entry must be consumed");
        assert!(rx.await.expect("first decision wins"));
    }

    #[tokio::test]
    async fn cancel_removes_the_entry_so_the_receiver_errors() {
        let gate = ApprovalGate::new();
        let rx = gate.register("a1");

        gate.cancel("a1");

        assert!(rx.await.is_err(), "sender should have been dropped");
        assert!(!gate.resolve("a1", true), "the entry is gone");
    }

    #[test]
    fn cancelling_an_unknown_id_is_a_no_op() {
        let gate = ApprovalGate::new();
        gate.cancel("never-registered");
    }

    #[test]
    fn resolving_after_the_receiver_is_dropped_still_reports_found() {
        // The runtime's 5-minute timeout drops its receiver; a late decision
        // must not be mistaken for "not found".
        let gate = ApprovalGate::new();
        let rx = gate.register("a1");
        drop(rx);

        assert!(gate.resolve("a1", true));
    }

    #[tokio::test]
    async fn concurrent_registrations_resolve_independently_and_out_of_order() {
        let gate = std::sync::Arc::new(ApprovalGate::new());
        let ids: Vec<String> = (0..50).map(|i| format!("a{i}")).collect();
        let receivers: Vec<_> = ids.iter().map(|id| gate.register(id)).collect();

        // Resolve in reverse, alternating the decision.
        for (i, id) in ids.iter().enumerate().rev() {
            assert!(gate.resolve(id, i % 2 == 0));
        }

        for (i, rx) in receivers.into_iter().enumerate() {
            assert_eq!(
                rx.await.expect("delivered"),
                i % 2 == 0,
                "approval {i} got the wrong decision"
            );
        }
    }

    #[tokio::test]
    async fn a_waiting_task_is_woken_by_a_later_resolve() {
        let gate = std::sync::Arc::new(ApprovalGate::new());
        let rx = gate.register("a1");

        let waiter = tokio::spawn(rx);

        // The decision arrives after the waiter is already parked.
        tokio::task::yield_now().await;
        assert!(gate.resolve("a1", true));

        assert!(waiter.await.expect("join").expect("delivered"));
    }
}
