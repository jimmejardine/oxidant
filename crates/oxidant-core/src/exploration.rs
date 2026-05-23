// Realises spec/components/core/exploration.md.
//
// The Exploration aggregate: a self-contained runtime workspace with its
// own conversation, branch, worktree, optional rust-analyzer handle, and
// cancellation token. The GUI viewport keys windows by ExplorationId;
// the agent loop runs per-Exploration.
//
// Two construction paths per the spec:
//   - new_main(workspace_root)            — built at app launch from
//                                            the repo's existing checkout.
//   - from_sub_worktree(parent_id, ...)   — built by the spawn-exploration
//                                            flow after worktree_mgmt produces
//                                            a fresh WorktreeHandle.
//
// LspHandle is a marker trait so the LSP layer can hang its concrete
// handle off here without oxidant-core depending on oxidant-rust-tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use ulid::Ulid;

use crate::conversation::Conversation;

// ---------------------------------------------------------------- id

/// Stable per-exploration identifier. ULID, so it sorts by creation time
/// and round-trips through transcripts, dock-layout files, and GUI window
/// keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExplorationId(pub Ulid);

impl ExplorationId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }

    pub fn nil() -> Self {
        Self(Ulid::nil())
    }

    pub fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl Default for ExplorationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ExplorationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for ExplorationId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(s).map(Self)
    }
}

// ---------------------------------------------------------------- kind

#[derive(Debug, Clone)]
pub enum ExplorationKind {
    /// The repo's existing checkout. There's exactly one Main per running
    /// process (the GUI's primary window).
    Main,
    /// A sub-exploration spawned off another exploration's worktree.
    Sub { parent_id: ExplorationId },
}

// ---------------------------------------------------------------- lsp handle

/// Marker trait so the LSP component (which lives in oxidant-rust-tools,
/// downstream of this crate) can stash its per-exploration handle here
/// without creating a circular dependency. The concrete impl plugs in
/// via `impl LspHandle for LspClient {}` in oxidant-rust-tools.
pub trait LspHandle: Send + Sync + std::fmt::Debug {}

// ---------------------------------------------------------------- aggregate

#[derive(Debug, Clone)]
pub struct Exploration {
    pub id: ExplorationId,
    pub kind: ExplorationKind,
    pub worktree_path: PathBuf,
    pub branch: String,
    pub conversation: Conversation,
    /// Populated lazily on the first LSP query. The component spec calls
    /// out the "spawned lazily" pattern explicitly.
    pub lsp_handle: Option<Arc<dyn LspHandle>>,
    /// `<worktree>/target` — split out so per-exploration tools can pin
    /// `CARGO_TARGET_DIR` without re-deriving it.
    pub target_dir: PathBuf,
    pub cancellation: CancellationToken,
    pub created_at: DateTime<Utc>,
}

impl Exploration {
    /// Build the Main exploration from the repo's existing checkout.
    /// Branch is taken from the current `git symbolic-ref` if available;
    /// callers can override after construction if they already have it.
    pub fn new_main(workspace_root: impl Into<PathBuf>, branch: impl Into<String>) -> Self {
        let worktree_path = workspace_root.into();
        let target_dir = worktree_path.join("target");
        Self {
            id: ExplorationId::new(),
            kind: ExplorationKind::Main,
            worktree_path,
            branch: branch.into(),
            conversation: Conversation::new(),
            lsp_handle: None,
            target_dir,
            cancellation: CancellationToken::new(),
            created_at: Utc::now(),
        }
    }

    /// Build a Sub exploration from the pieces the spawn-exploration flow
    /// has already assembled: the parent's id, the new WorktreeHandle's
    /// path + branch.
    pub fn from_sub_worktree(
        parent_id: ExplorationId,
        worktree_path: impl Into<PathBuf>,
        branch: impl Into<String>,
    ) -> Self {
        let worktree_path = worktree_path.into();
        let target_dir = worktree_path.join("target");
        Self {
            id: ExplorationId::new(),
            kind: ExplorationKind::Sub { parent_id },
            worktree_path,
            branch: branch.into(),
            conversation: Conversation::new(),
            lsp_handle: None,
            target_dir,
            cancellation: CancellationToken::new(),
            created_at: Utc::now(),
        }
    }

    pub fn is_main(&self) -> bool {
        matches!(self.kind, ExplorationKind::Main)
    }

    pub fn parent_id(&self) -> Option<ExplorationId> {
        match self.kind {
            ExplorationKind::Main => None,
            ExplorationKind::Sub { parent_id } => Some(parent_id),
        }
    }

    pub fn worktree_path(&self) -> &Path {
        &self.worktree_path
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// Trigger cooperative cancellation. The agent loop short-circuits at
    /// the next yield; in-flight tool calls see `ctx.is_cancelled()` and
    /// abort.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

// ---------------------------------------------------------------- summary

/// Lightweight view used by the GUI's exploration list and the
/// session-persistence restore pass. Doesn't carry the conversation,
/// LSP handle, or cancellation token — those live in the in-memory
/// Exploration once the user opens the window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorationSummary {
    pub id: ExplorationId,
    pub kind_label: String,
    pub branch: String,
    pub worktree_path: PathBuf,
    pub last_seen: DateTime<Utc>,
    pub message_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_main_is_marked_main_and_has_target_under_worktree() {
        let e = Exploration::new_main("/tmp/example", "main");
        assert!(e.is_main());
        assert_eq!(e.parent_id(), None);
        assert_eq!(e.target_dir, PathBuf::from("/tmp/example/target"));
        assert_eq!(e.branch, "main");
    }

    #[test]
    fn sub_carries_parent_id_and_isnt_main() {
        let parent = ExplorationId::new();
        let e = Exploration::from_sub_worktree(parent, "/tmp/sub", "oxidant/explore/x");
        assert!(!e.is_main());
        assert_eq!(e.parent_id(), Some(parent));
        assert_eq!(e.branch, "oxidant/explore/x");
    }

    #[test]
    fn cancel_token_flips_is_cancelled() {
        let e = Exploration::new_main("/tmp/c", "main");
        assert!(!e.is_cancelled());
        e.cancel();
        assert!(e.is_cancelled());
    }

    #[test]
    fn ids_roundtrip_through_string() {
        let id = ExplorationId::new();
        let s = id.to_string();
        let parsed: ExplorationId = s.parse().expect("parses back");
        assert_eq!(id, parsed);
    }

    #[test]
    fn ids_are_unique_across_calls() {
        let a = ExplorationId::new();
        let b = ExplorationId::new();
        assert_ne!(a, b);
    }
}
