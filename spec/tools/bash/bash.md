```yaml
---
id: bash
kind: tool
parent: components/tools/bash-runner
order: 1
implements:
  - contracts/tool
depends_on:
  - components/tools/bash-runner
code:
  - crates/oxidant-tools/src/bash.rs
status: active
responsibility: |
  Run a shell command in the worktree with a timeout and captured output; the escape hatch when no first-class tool covers the case.
---
```

`category`: `Mutating`.

## Schema

```json
{
  "type": "object",
  "required": ["command"],
  "properties": {
    "command":     { "type": "string", "description": "passed to the shell as-is" },
    "timeout_ms":  { "type": "integer", "default": 120000, "maximum": 600000 },
    "stdin":       { "type": "string", "description": "optional stdin" }
  }
}
```

## Result

```json
{
  "exit_code": 0,
  "stdout":    "...",
  "stderr":    "...",
  "stdout_truncated": false,
  "stderr_truncated": false,
  "duration_ms": 234
}
```

## Shell

- Windows: `cmd.exe /S /C` (default) or PowerShell when configured.
- Unix: `bash -c`.

Environment carries oxidant's injected vars (`RUSTC_WRAPPER`, `CARGO_TARGET_DIR`, others from settings).

## Output limits

stdout/stderr capped at 30KB each by default (tail-truncated with marker). Override via settings.

## When to use

Prefer first-class tools first: cargo, lsp, syn, fs, vcs. Use `bash` only when none cover the case (e.g. shelling out to `cargo audit`, `wasm-pack`, custom scripts).
