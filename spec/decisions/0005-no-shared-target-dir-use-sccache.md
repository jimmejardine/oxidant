```yaml
id: 0005-no-shared-target-dir-use-sccache
kind: decision
order: 5
status: active
date: 2026-05-21
responsibility: |
  Each worktree keeps its own target/; cross-worktree compile reuse comes from sccache, not shared CARGO_TARGET_DIR.
```

# 0005 — Per-worktree `target/` plus sccache, no shared `CARGO_TARGET_DIR`

## Status

Active.

## Context

With one worktree per exploration ([[decisions/0004-git-worktree-per-exploration]]), naively each gets its own multi-GB `target/`. Disk grows linearly with active explorations.

The obvious fix — set `CARGO_TARGET_DIR` to a shared path — has a hidden cost: cargo takes a process-wide lock on `target/`, serialising builds across all worktrees pointing at it. Parallel build capacity goes to zero. For an agent that wants to run `cargo check` in multiple explorations concurrently, that's a disaster.

`sccache` (Mozilla's distributed compilation cache) caches rustc outputs keyed by source + flags + sysroot. Multiple worktrees building the same crate versions hit cache, regardless of where their `target/` lives.

## Decision

- Each worktree has its own `target/`, inside the worktree (`<worktree>/target/`).
- `CARGO_TARGET_DIR` is set explicitly per cargo invocation, never to a shared path.
- `RUSTC_WRAPPER=sccache` is set in the environment of every cargo subprocess spawned by [[components/rust-tools/cargo-runner]].
- `sccache` is an assumed external binary; hard-fail at launch if absent.

Sccache uses its default cache directory (shared with the user's other Rust work, which is a feature).

## Consequences

Positive: parallel builds across explorations work; disk usage stays bounded by sccache's cache eviction; benefits propagate to the user's other Rust projects.

Negative: another required external binary. Mitigation: clear launch-time check with install instructions. If the user already has `RUSTC_WRAPPER` set (e.g. for `cranelift`), prompt rather than overwrite.

## Related

- [[invariants/target-dirs-are-never-shared]]
- [[components/rust-tools/cargo-runner]] — sets the env vars
