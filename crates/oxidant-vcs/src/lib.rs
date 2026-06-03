// Git shellout, worktree management for sub-explorations, session persistence.
//
// Specs:
//   spec/components/vcs/git-shellout.md
//   spec/components/vcs/worktree-mgmt.md
//   spec/components/vcs/session-persistence.md
//   spec/decisions/0004-git-worktree-per-exploration.md
//   spec/decisions/0006-shell-out-to-git-cli.md
//   spec/invariants/explorations-are-isolated.md
//   spec/invariants/target-dirs-are-never-shared.md

use std::sync::Arc;

use oxidant_core::{Tool, ToolRegistry};

pub mod git;
pub mod session_persist;
pub mod tools;
pub mod worktree;

pub use git::{
    BranchCreateOutcome, CheckoutOutcome, Commit, CommitOutcome, DiffFile, DiffHunk, DiffOutput,
    Git, GitError, LogOpts, MergeOpts, MergeOutcome, StatusFile, StatusOutput, Worktree,
};
pub use session_persist::{SessionError, SessionMeta, SessionSummary};
pub use tools::{
    VcsBranchCheckout, VcsBranchCreate, VcsCommit, VcsDiff, VcsDiscard, VcsExplorationsList,
    VcsExplore, VcsLog, VcsMergeBack, VcsStatus,
};
pub use worktree::{MergeBackOpts, SpawnOpts, WorktreeError, WorktreeHandle, WorktreeSummary};

/// Register the seven agent-safe VCS tools (status, diff, log, commit,
/// branch-create, branch-checkout, explorations-list). The three
/// GUI-only ones (vcs_explore, vcs_discard, vcs_merge_back) are
/// deliberately omitted — they return ToolResult::Err if called from
/// the agent, and the GUI registers them separately when it ships.
pub fn register_standard_tools(registry: &mut ToolRegistry) {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(VcsStatus),
        Arc::new(VcsDiff),
        Arc::new(VcsLog),
        Arc::new(VcsCommit),
        Arc::new(VcsBranchCreate),
        Arc::new(VcsBranchCheckout),
        Arc::new(VcsExplorationsList),
    ];
    for t in tools {
        registry.register(t);
    }
}

/// Register the three GUI-only VCS tools. Call this only from the GUI
/// process, NOT from the agent example. Each tool returns an error if
/// invoked, so even if it's mis-registered the agent gets a clear
/// refusal rather than a runtime spawn.
pub fn register_gui_only_tools(registry: &mut ToolRegistry) {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(VcsExplore),
        Arc::new(VcsDiscard),
        Arc::new(VcsMergeBack),
    ];
    for t in tools {
        registry.register(t);
    }
}
