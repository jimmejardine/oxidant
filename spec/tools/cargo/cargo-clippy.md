---
id: cargo-clippy
kind: tool
parent: components/rust-tools/cargo-runner
order: 4
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/cargo-runner
code:
  - crates/oxidant-rust-tools/src/cargo_runner.rs
status: active
responsibility: |
  Run clippy with --message-format=json and return structured lint diagnostics; optionally apply machine-applicable suggestions.
---

`category`: `ReadOnly` by default; `Mutating` when `fix: true`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "package":     { "type": "string" },
    "fix":         { "type": "boolean", "default": false, "description": "apply machine-applicable suggestions" },
    "deny":        { "type": "array", "items": { "type": "string" }, "description": "lints to deny" },
    "allow":       { "type": "array", "items": { "type": "string" } }
  }
}
```

## Result

Same shape as [[tools/cargo/cargo-check]] but `level` includes lint levels (`note`, `help`) and `code` is the lint name (e.g. `clippy::needless_clone`). When `fix: true`, clippy applies suggestions to source files; oxidant then runs syn-parse validation over the touched files (same invariant as the edit substrate) before reporting success.

## See also

- [[tools/edit/apply-edits]] — to apply individual suggestions selectively
- [[tools/cargo/cargo-check]]
