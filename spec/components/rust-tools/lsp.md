```yaml
id: lsp
kind: component
parent: overview
order: 1
implements: []
depends_on:
  - contracts/workspace-edit
code:
  - crates/oxidant-rust-tools/src/lsp_client.rs
tests:
  - crates/oxidant-rust-tools/src/lsp_client.rs
  - crates/oxidant-rust-tools/tests/lsp_live.rs
status: active
responsibility: |
  Manage one rust-analyzer process per exploration over async-lsp and expose its capabilities as structured agent tools.
```

The semantic spine. rust-analyzer runs as a subprocess per active exploration ([[decisions/0009-no-ra-ap-crates-lsp-suffices]]); this component owns the lifecycle and serves the tool surface in [[tools/lsp/rust-hover]], [[tools/lsp/rust-goto-definition]], etc.

## Lifecycle

- **Spawn**: Lazily on first LSP query in an exploration. Path resolution: `rustup which rust-analyzer` first, else `which rust-analyzer`. Hard fail at launch if neither succeeds (see [[decisions/0009-no-ra-ap-crates-lsp-suffices]]).
- **Initialize**: Standard LSP `initialize` with `workspaceFolders = [worktree_path]`. Wait for `initialized` notification before serving requests.
- **Idle**: Process stays alive for the exploration's lifetime. v2 may add idle eviction.
- **Shutdown**: `shutdown` + `exit` on exploration close. Force-kill on timeout (5s).

## Path conventions

LSP works in `file://` URIs. On Windows, paths must be canonicalised through `dunce` and encoded as `file:///C:/...` (drive letter, forward slashes, no UNC). A `LspUri` newtype owns this conversion — surfaced via [[invariants/explorations-are-isolated]] context.

## Request shapes (oxidant → LSP)

Common operations:
- `textDocument/hover` → [[tools/lsp/rust-hover]]
- `textDocument/definition` → [[tools/lsp/rust-goto-definition]]
- `textDocument/references` → [[tools/lsp/rust-find-references]]
- `textDocument/rename` → [[tools/lsp/rust-rename]] (returns `WorkspaceEdit`)
- `textDocument/codeAction` → [[tools/lsp/rust-code-actions]]
- `workspace/symbol` → [[tools/lsp/rust-workspace-symbols]]
- `textDocument/publishDiagnostics` (server-pushed) → [[tools/lsp/rust-diagnostics]] (most recent set cached per file)

## Coordinate system

LSP returns UTF-16 code-unit columns. Conversion to byte offsets for the edit substrate happens at the boundary in [[components/tools/workspace-edit-substrate]].
