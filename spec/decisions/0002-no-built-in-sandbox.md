```yaml
id: 0002-no-built-in-sandbox
kind: decision
order: 2
status: active
date: 2026-05-21
responsibility: |
  Oxidant does not implement an in-process sandbox; users run oxidant under whatever isolation they prefer (devcontainer, VM, none).
```

# 0002 — Oxidant ships without a built-in sandbox

## Status

Active.

## Context

LLM-generated code can be wrong in destructive ways: deleting files, exfiltrating env vars, hitting network endpoints. `build.rs` makes `cargo check` itself a code-execution vector — a malicious crate dep can pwn your machine. A real sandbox materially raises the trust ceiling.

But sandboxing is also non-trivial: Docker dependency, mount semantics, `target/` placement, WSL filesystem perf on Windows, per-platform mechanisms (bubblewrap/landlock/seatbelt/job-objects). Claude Code's default is *no sandbox* — it uses a permission-prompt model. That model has worked.

## Decision

Oxidant ships with **no built-in sandbox**. Tool calls run as the user's process with the user's privileges. Permission prompts (read-only auto-approve, mutating prompts unless allowlisted) provide a coarse safety layer, but they are not a security boundary.

Users who want isolation are expected to run oxidant inside their preferred mechanism: devcontainer, VM, fresh user account, dedicated workstation, etc. The `oxidant-config` permission-prompt UX is the explicit safety story.

## Consequences

Positive: zero sandbox complexity; works the same everywhere; no Docker/WSL/landlock branching.

Negative: a malicious crate dep can compromise the host. Users running oxidant on prod-adjacent machines should not. We document this in the README's "Threat model" section and surface it once on first run.

## Related

- [[components/config/permissions]] — the permission-prompt layer, which is not the same thing as a sandbox
