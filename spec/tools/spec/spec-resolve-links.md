---
id: spec-resolve-links
kind: tool
parent: components/spec-tools/graph
order: 3
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/graph
code:
  - crates/oxidant-spec-tools/src/tools/spec_resolve_links.rs
status: active
responsibility: |
  Return all inbound and outbound links (frontmatter and body refs) for one spec.
---

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["ref"],
  "properties": {
    "ref": { "type": "string" }
  }
}
```

## Result

```json
{
  "inbound": [
    { "from": "flows/fix-diagnostic", "edge": "depends_on" },
    { "from": "components/tools/edit", "edge": "body_ref" }
  ],
  "outbound": [
    { "to": "components/tools/workspace-edit-substrate", "edge": "depends_on" },
    { "to": "invariants/edits-are-atomic", "edge": "body_ref" }
  ]
}
```

## Use cases

- "Who references this spec?" — orientation when modifying it.
- "What does this spec depend on?" — impact analysis when its dependencies change.
- Drives the GUI's spec sidebar "incoming/outgoing" section.
