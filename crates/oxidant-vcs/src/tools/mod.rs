// Model-facing tool wrappers for the VCS components.
//
// Agent-safe (registered by default):
//   vcs_status, vcs_diff, vcs_log, vcs_commit,
//   vcs_branch_create, vcs_branch_checkout, vcs_explorations_list
//
// GUI-only (not registered for the agent — they refuse with an error
// at invoke time, and aren't included in register_standard_tools):
//   vcs_explore, vcs_discard, vcs_merge_back

pub mod vcs_branch_checkout;
pub mod vcs_branch_create;
pub mod vcs_commit;
pub mod vcs_diff;
pub mod vcs_discard;
pub mod vcs_explore;
pub mod vcs_explorations_list;
pub mod vcs_log;
pub mod vcs_merge_back;
pub mod vcs_status;

pub use vcs_branch_checkout::VcsBranchCheckout;
pub use vcs_branch_create::VcsBranchCreate;
pub use vcs_commit::VcsCommit;
pub use vcs_diff::VcsDiff;
pub use vcs_discard::VcsDiscard;
pub use vcs_explore::VcsExplore;
pub use vcs_explorations_list::VcsExplorationsList;
pub use vcs_log::VcsLog;
pub use vcs_merge_back::VcsMergeBack;
pub use vcs_status::VcsStatus;
