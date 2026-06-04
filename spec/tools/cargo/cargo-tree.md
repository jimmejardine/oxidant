```yaml
id: cargo-tree
kind: tool
parent: components/rust-tools/cargo-runner
order: 6
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/cargo-runner
code:
  - crates/oxidant-rust-tools/src/cargo_runner.rs
status: active
responsibility: |
  Return the cargo dependency tree as structured nodes (package, version, source, dependents).
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "package":   { "type": "string", "description": "root; omit for workspace root" },
    "depth":     { "type": "integer", "default": 5, "maximum": 20 },
    "no_dedupe": { "type": "boolean", "default": false }
  }
}
```

## Result

```json
{
  "root": {
    "name": "oxidant-tools",
    "version": "0.1.0",
    "dependencies": [
      { "name": "syn", "version": "2.0.117", "source": "registry+https://crates.io/", "dependencies": [] }
    ]
  }
}
```

## Implementation

Internally runs `cargo metadata` (more structured than `cargo tree`) and walks the resolve graph. Cheaper and gives richer info than parsing `cargo tree` text output.

## See also

- [[tools/cargo/cargo-metadata]] — flat metadata view rather than tree
