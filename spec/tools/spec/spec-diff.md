---
id: spec-diff
kind: tool
parent: components/spec-tools/diff
order: 6
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/diff
code:
  - crates/oxidant-spec-tools/src/tools/spec_diff.rs
tests:
  - crates/oxidant-spec-tools/tests/diff_real_tree.rs
status: active
responsibility: |
  Detect spec↔code drift; in MVP this means trait-method drift for contract specs and missing code: paths for component specs.
---

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "ref": { "type": "string", "description": "limit to this spec; omit for tree-wide" }
  }
}
```

## Result

```json
{
  "drifts": [
    { "kind": "MethodSignatureChanged",
      "contract_id": "contracts/tool",
      "method": "invoke",
      "spec": "async fn invoke(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult",
      "code": "async fn invoke(&self, args: serde_json::Value) -> ToolResult" },
    { "kind": "MissingCodePath",
      "spec_id": "components/tools/edit",
      "path":    "crates/oxidant-tools/src/edit.rs" }
  ]
}
```

## When this runs

- On every successful agent edit ([[components/core/agent-loop]] post-commit hook).
- On demand from the GUI.
- Pre-merge in [[flows/merge-back]] — drift should be resolved before merge.

## Deferred scope

Responsibility-vs-impl semantic drift, flow-vs-call-graph drift, invariant verification — all v2. See [[components/spec-tools/diff]] for what's not yet covered.
