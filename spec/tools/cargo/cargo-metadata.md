```yaml
id: cargo-metadata
kind: tool
parent: components/rust-tools/cargo-runner
order: 7
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/cargo-runner
code:
  - crates/oxidant-rust-tools/src/cargo_runner.rs
status: active
responsibility: |
  Return workspace metadata: members, dependencies, features, target_dir, rust-version.
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "no_deps":  { "type": "boolean", "default": false, "description": "skip resolving dependencies" },
    "features": { "type": "array", "items": { "type": "string" } }
  }
}
```

## Result

A pared-down version of cargo's own `cargo metadata` JSON: workspace members with their dependencies, features, and targets. Used by the agent for orientation (`"list the workspace members"`) and by [[components/spec-tools/diff]] to find code files referenced in spec `code:` fields.

## Implementation

Wraps the `cargo_metadata` crate; results are deserialised into typed structs and re-serialised to a stable shape (the crate's own struct shape changes over time).
