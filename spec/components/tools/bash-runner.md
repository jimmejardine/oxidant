```yaml
---
id: bash-runner
kind: component
parent: overview
order: 2
implements: []
depends_on: []
code:
  - crates/oxidant-tools/src/bash.rs
status: active
responsibility: |
  Execute shell commands in the exploration's working directory with a timeout, captured stdout/stderr, and structured output.
---
```

The escape hatch. When no first-class tool covers what the model needs, [[tools/bash/bash]] runs the command directly.

## Behaviour

- Working directory: `ToolContext::workspace_root`.
- Shell on Windows: `cmd.exe /S /C` (default) or PowerShell when configured.
- Shell on Unix: `bash -c`.
- Timeout: 120s default, 600s max, configurable per-call.
- Output capture: stdout + stderr separately, byte limit (default 30KB; tail-truncated with marker if exceeded).
- Environment: inherits agent env plus oxidant's injected vars (`RUSTC_WRAPPER=sccache`, `CARGO_TARGET_DIR=<worktree>/target`).

## Permission category

`Mutating` — bash can do anything; permission prompt is the only safety. Heuristic allowlist in [[components/config/permissions]] (e.g. `ls`, `pwd`, `cat` auto-approve; `rm`, `curl` always prompt).

## Why not a richer abstraction

Modelling bash as anything more structured (parsed args, typed return) defeats the purpose — the model uses bash precisely when the structured tools don't cover the case. Keep it raw and rely on the model to use the first-class Rust tools first.
