```yaml
id: spec-search
kind: tool
parent: components/spec-tools/index-db
order: 2
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/index-db
code:
  - crates/oxidant-spec-tools/src/tools/spec_search.rs
status: active
responsibility: |
  Structured query over spec metadata (kind, status, parent, edges) without free-text matching.
```

The structural counterpart to [[tools/search/text-search]]. Answers questions like "list all draft components", "which tools implement contracts/provider", "find specs with no inbound references". Hits the SQLite index ([[components/spec-tools/index-db]]) directly — no text matching.

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "kind":          { "type": "string", "description": "filter by kind" },
    "status":        { "type": "string", "enum": ["draft", "active", "deprecated"] },
    "parent":        { "type": "string", "description": "canonical ref of parent" },
    "implements":    { "type": "string", "description": "canonical ref of a contract" },
    "depends_on":    { "type": "string", "description": "canonical ref" },
    "depended_by":   { "type": "string", "description": "canonical ref" },
    "orphans":       { "type": "boolean", "description": "specs with no inbound edges" },
    "limit":         { "type": "integer", "default": 50, "maximum": 500 }
  },
  "additionalProperties": false
}
```

## Result shape

```json
{
  "rows": [
    {
      "id": "tools/edit/apply-edits",
      "kind": "tool",
      "parent": "components/tools/edit",
      "status": "active",
      "last_modified": "2026-05-21T14:02:11Z"
    }
  ],
  "count": 1
}
```

## Composition examples

- "all draft tools": `{ kind: "tool", status: "draft" }`
- "everything depending on workspace-edit-substrate": `{ depends_on: "components/tools/workspace-edit-substrate" }`
- "orphaned components": `{ kind: "component", orphans: true }`

## See also

- [[tools/search/text-search]] — free-text BM25
- [[tools/spec/spec-tree]] — hierarchical view rather than flat query
- [[tools/spec/spec-resolve-links]] — single-spec link details
