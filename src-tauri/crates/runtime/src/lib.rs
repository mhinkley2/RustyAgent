pub mod approval_gate;
pub mod context;
pub mod git;
pub mod notifier;
pub mod permission;
pub mod runtime;
pub mod worktree;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod context_tests;

#[cfg(test)]
mod permission_tests;

#[cfg(test)]
mod runtime_tests;

#[cfg(test)]
mod worktree_tests;

pub use approval_gate::ApprovalGate;
pub use notifier::AppNotifier;
pub use context::{ContextPolicy, ContextStrategy};
pub use permission::{PermissionPolicy, PolicyDecision, ToolRequest};
pub use runtime::{ApprovalOutcome, CancelFlag, ConversationRuntime, RunEvent};
pub use worktree::{Isolation, RunWorktree};
