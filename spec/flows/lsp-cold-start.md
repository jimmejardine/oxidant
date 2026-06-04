```yaml
---
id: lsp-cold-start
kind: flow
parent: overview
order: 8
status: active
responsibility: |
  First LSP-backed tool call in an exploration spawns rust-analyzer, sends `initialize`, and waits for `experimental/serverStatus { quiescent: true }` before issuing semantic queries. Subsequent calls reuse the cached client. Documents the latency surprise.
depends_on:
  - components/rust-tools/lsp
  - tools/lsp/rust-hover
  - tools/lsp/rust-goto-definition
  - tools/lsp/rust-find-references
  - tools/lsp/rust-workspace-symbols
  - tools/lsp/rust-rename
  - tools/lsp/rust-code-actions
  - tools/lsp/rust-diagnostics
  - decisions/0009-no-ra-ap-crates-lsp-suffices
---
```

# Cold-start rust-analyzer for an exploration

Why the first LSP tool call in a fresh worktree takes 10-30 seconds and every call after is fast. Operational background, not a thing the agent has to do — the latency is built into the LSP component.

## Trigger

Any of the `rust_*` tools fires for the first time inside a `workspace_root` whose `LspClient` isn't already cached: [[tools/lsp/rust-hover]], [[tools/lsp/rust-goto-definition]], [[tools/lsp/rust-find-references]], [[tools/lsp/rust-workspace-symbols]], [[tools/lsp/rust-rename]], [[tools/lsp/rust-code-actions]], [[tools/lsp/rust-diagnostics]].

## Steps

1. **Cache lookup.** [[components/rust-tools/lsp]]::`LspClient::for_workspace(workspace)` canonicalises the path and looks it up in the process-global `LSP_CLIENTS` map. Hit → return the cached `Arc<AsyncMutex<LspClient>>`; the tool proceeds at warm-call latency (single-digit ms).

2. **Spawn the subprocess.** Cache miss: locate `rust-analyzer` on PATH (`rustup component add rust-analyzer` is the documented prerequisite). Spawn it with stdin/stdout/stderr piped, `current_dir = workspace`, `kill_on_drop = true`. If spawn fails, the tool returns `ToolResult::Err("spawn rust-analyzer failed: …")` and the cache stays empty so the next call retries.

3. **Wire the JSON-RPC tasks.** Two background tokio tasks:
   - **Writer**: drains an `mpsc::UnboundedReceiver<String>` to stdin, framed with the LSP `Content-Length:` header.
   - **Reader**: parses stdout, routes responses to per-id `oneshot::Sender<Value>` slots, stashes `publishDiagnostics` into a `HashMap<PathBuf, Vec<Value>>` cache, updates an `experimental/serverStatus` `watch::channel`.
   - **stderr drain**: traced for debugging, never blocks.

4. **`initialize` round-trip.** Send a JSON-RPC `initialize` request with `workspaceFolders`, `rootUri`, and minimal client capabilities. Await the response with a 60s timeout (`INITIALIZE_TIMEOUT_SECS`). Timeout → tear down the subprocess and return an error.

5. **`initialized` notification.** Per the LSP spec; tells rust-analyzer it can begin its own work.

6. **Wait for quiescent.** rust-analyzer emits `experimental/serverStatus { health, quiescent, message? }` notifications during indexing. The first tool call awaits `quiescent: true` on the `watch::Receiver` with a 60s budget (`READY_TIMEOUT_SECS`). Cold start on a fresh tempdir crate: 10-30 seconds typical; large workspaces with many dependencies: up to the full budget. Timing out here returns an error and leaves the (now-spawned) client cached — the next call will skip respawn but reuse the existing wait machinery.

7. **Insert into cache.** With the client ready, wrap in `Arc<AsyncMutex<...>>` and insert under the canonicalised workspace path. The whole spawn-and-wait sequence is racy across concurrent first-call tool invocations; the cache insert uses `entry().or_insert(arc).clone()` so the loser drops its spare client and the winner is shared.

8. **Tool proceeds.** From now on, the tool calls `client.request(method, params)` and gets a structured response back — no further cold-start cost for the lifetime of the process.

## Why this cost is unavoidable

rust-analyzer's first tool call requires it to have built its analysis database for the crate. That means: running `cargo metadata`, resolving dependencies, parsing every Rust file in scope, building the salsa-style incremental index. There is no faster path — bypassing rust-analyzer ([[decisions/0009-no-ra-ap-crates-lsp-suffices]] considers this and rejects it) would mean rebuilding the same database ourselves without the existing tooling. The `experimental/serverStatus { quiescent: true }` signal exists precisely because semantic queries against a half-built index return wrong answers.

## Common failure modes

- **`rust-analyzer` not on PATH.** Spawn fails immediately. Surface as `ToolResult::Err`; the GUI's first hover/goto attempt will show the message. Documented in the README's Environment section.
- **Cold start exceeds 60s.** Usually a very large dependency graph or a slow disk. The cached client survives the timeout; a subsequent call within ~30s will likely find it already quiescent. Bumping `READY_TIMEOUT_SECS` is reasonable on machines that consistently exceed it.
- **rust-analyzer crashes mid-session.** Reader task notices `stdout` close. The cached client becomes broken — subsequent requests time out at `REQUEST_TIMEOUT_SECS` (30s). Recovery is process-restart (no automatic respawn in MVP); future hardening could detect the broken state and evict the cache entry.

## See also

- [[components/rust-tools/lsp]] — full implementation, including the JSON-RPC reader and the diagnostics cache
- [[decisions/0009-no-ra-ap-crates-lsp-suffices]] — why we accept the cold-start cost
