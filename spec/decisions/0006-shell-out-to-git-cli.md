```yaml
---
id: 0006-shell-out-to-git-cli
kind: decision
order: 6
status: active
date: 2026-05-21
responsibility: |
  oxidant-vcs shells out to the git CLI rather than linking git2 or gix.
---
```

# 0006 — Shell out to the `git` CLI

## Status

Active.

## Context

Three ways to invoke git from Rust:

- **`git2`** (libgit2 bindings): mature, capable, but a C dependency that complicates cross-compilation and increases binary size.
- **`gix`** (gitoxide, pure Rust): excellent and fast, but evolving — frequent breaking releases, not yet 1.0.
- **Shell out to `git`**: trivial, capability-complete, version-agnostic, and the user already has it.

oxidant's git surface is small: status, diff, worktree add/list/remove, branch create/checkout, commit, merge, log. None of these need the inner-loop performance characteristics that justify in-process git.

## Decision

`oxidant-vcs` shells out to the `git` CLI for all operations. `git` is an assumed external binary; hard-fail at launch if missing.

This is part of a broader pattern (see [[decisions/0005-no-shared-target-dir-use-sccache]] for `sccache`, [[decisions/0009-no-ra-ap-crates-lsp-suffices]] for `rust-analyzer`): oxidant coordinates mature external tools rather than reimplementing them.

## Consequences

Positive: zero git dep churn; rides git's own improvements for free; cross-platform consistency.

Negative: subprocess overhead (~5–20ms per call). Acceptable — agent-driven git ops are not on a hot path.

## Related

- [[components/vcs/git-shellout]] — implementation
- [[decisions/0005-no-shared-target-dir-use-sccache]]
- [[decisions/0009-no-ra-ap-crates-lsp-suffices]]
