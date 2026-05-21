---
id: target-dirs-are-never-shared
kind: invariant
order: 4
status: active
depends_on:
  - components/rust-tools/cargo-runner
responsibility: |
  Each exploration's cargo runs against its own per-worktree target/ directory; CARGO_TARGET_DIR is never set to a shared path.
---

Every cargo subprocess spawned by [[components/rust-tools/cargo-runner]] runs with `CARGO_TARGET_DIR=<worktree>/target` explicitly. No exploration shares its `target/` with any other.

Cross-worktree compile reuse is achieved via `sccache` ([[decisions/0005-no-shared-target-dir-use-sccache]]), not via shared target directories.

## Why this invariant matters

Cargo takes a process-wide lock on `target/`. A shared `CARGO_TARGET_DIR` would serialise builds across all explorations pointing at it — for an agent that wants to run `cargo check` in multiple explorations concurrently, this collapses parallel build capacity to zero.

## Enforcement

- `oxidant-rust-tools/cargo_runner.rs` injects `CARGO_TARGET_DIR=<worktree>/target` on every cargo invocation, overriding any inherited value.
- Configuration validation rejects any oxidant.toml setting that attempts to set a shared target dir (no such setting exists; this is a forward guard).
- Tests assert that two cargo invocations from two explorations don't block on each other.
