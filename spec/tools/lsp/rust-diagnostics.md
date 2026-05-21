---
id: rust-diagnostics
kind: tool
parent: components/rust-tools/lsp
order: 7
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/lsp
code:
  - crates/oxidant-rust-tools/src/lsp_client.rs
status: active
responsibility: |
  Return rust-analyzer's current diagnostics for one file or the whole workspace, from its push-published cache.
---

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "file":  { "type": "string", "description": "omit for workspace-wide" },
    "severity": { "type": "string", "enum": ["error", "warning", "info", "hint"] }
  }
}
```

## Result

```json
{
  "diagnostics": [
    { "file": "src/foo.rs",
      "range": { ... },
      "severity": "error",
      "message": "cannot find type `Foo` in this scope",
      "source": "rust-analyzer",
      "code": "E0412" }
  ]
}
```

## Source

rust-analyzer push-publishes diagnostics via `textDocument/publishDiagnostics`. [[components/rust-tools/lsp]] caches the latest set per file; this tool returns from cache. No on-demand re-analysis triggered.

## Comparison to cargo_check

- `rust_diagnostics` — fast, reflects RA's current understanding, may lag a few hundred ms behind a file edit
- `cargo_check` — authoritative compiler view, slow, full type-check
- Use `rust_diagnostics` for live feedback; `cargo_check` to confirm before commit
