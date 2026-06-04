```yaml
id: cargo-expand
kind: tool
parent: components/rust-tools/cargo-runner
order: 5
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/cargo-runner
code:
  - crates/oxidant-rust-tools/src/cargo_runner.rs
status: active
responsibility: |
  Run cargo-expand to show macro-expanded source for a target; advertise availability at startup.
```

`category`: `ReadOnly`.

## Availability

Requires `cargo-expand` to be installed. Oxidant probes for it at startup; if missing, the tool registers but `invoke` returns `{ available: false, install_hint: "cargo install cargo-expand" }`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "package":   { "type": "string" },
    "module":    { "type": "string", "description": "module path within the package, e.g. agent::loop" },
    "test":      { "type": "boolean", "default": false, "description": "expand test code" }
  }
}
```

## Result

```json
{ "available": true, "expanded": "...", "elapsed_ms": 4321 }
```

## Notes

- Useful for debugging proc-macro behaviour (`#[derive(...)]`, `#[tokio::main]`, etc.).
- Output can be large; uses the same 30KB-tail-truncation as bash, configurable.
