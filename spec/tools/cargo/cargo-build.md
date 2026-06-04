```yaml
id: cargo-build
kind: tool
parent: components/rust-tools/cargo-runner
order: 2
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/cargo-runner
code:
  - crates/oxidant-rust-tools/src/cargo_runner.rs
status: active
responsibility: |
  Run cargo build (debug or release) and return diagnostics plus produced artifact paths.
```

`category`: `Mutating` (writes binaries to `target/`).

## Schema

```json
{
  "type": "object",
  "properties": {
    "release":   { "type": "boolean", "default": false },
    "package":   { "type": "string" },
    "target":    { "type": "string", "description": "rustc target triple" },
    "features":  { "type": "array", "items": { "type": "string" } }
  }
}
```

## Result

```json
{
  "ok": true,
  "messages": [...],
  "artifacts": [
    { "package_id": "oxidant-gui 0.1.0", "kind": "bin", "path": "target/debug/oxidant" }
  ],
  "summary": { "errors": 0, "warnings": 0, "elapsed_ms": 18432 }
}
```

## Notes

- Honours sccache via `RUSTC_WRAPPER` set by [[components/rust-tools/cargo-runner]].
- For "did the workspace compile?" prefer [[tools/cargo/cargo-check]] — faster.
- Long-running; respects `ctx.cancellation`.
