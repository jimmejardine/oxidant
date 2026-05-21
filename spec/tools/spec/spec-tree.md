---
id: spec-tree
kind: tool
parent: components/spec-tools/graph
order: 2
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/graph
code:
  - crates/oxidant-spec-tools/src/tools/spec_tree.rs
status: active
responsibility: |
  Return a hierarchical view of the spec graph rooted at a given ref, walking parent or depends_on edges.
---

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "from_ref":  { "type": "string", "default": "overview" },
    "depth":     { "type": "integer", "default": 4, "maximum": 12 },
    "edge_kind": { "type": "string", "enum": ["parent", "depends_on"], "default": "parent" }
  }
}
```

## Result

```json
{
  "root": {
    "ref":  "overview",
    "kind": "overview",
    "children": [
      { "ref": "components/core/agent-loop", "kind": "component", "children": [...] }
    ]
  }
}
```

## Edge kinds

- `parent` (default): the hierarchical parent relationship encoded in frontmatter
- `depends_on`: the dependency tree — useful for impact analysis ("what hangs off the workspace-edit-substrate")
