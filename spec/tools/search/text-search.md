```yaml
id: text-search
kind: tool
parent: components/spec-tools/search-index
order: 1
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/search-index
code:
  - crates/oxidant-spec-tools/src/tools/text_search.rs
status: active
responsibility: |
  Full-text BM25 search across both spec markdown and Rust source, with optional source/kind/language filters.
```

The agent's primary "find a concept" tool. Hits both `spec/**/*.md` and `crates/**/*.rs` in one call, with optional filters to narrow the scope.

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["query"],
  "properties": {
    "query": { "type": "string", "minLength": 1 },
    "source": { "enum": ["spec", "code", "both"], "default": "both" },
    "kind":   { "type": "string", "description": "spec kind filter (only when source = spec or both)" },
    "lang":   { "type": "string", "description": "language filter (only when source = code or both)" },
    "limit":  { "type": "integer", "default": 20, "minimum": 1, "maximum": 100 }
  }
}
```

## Result shape

```json
{
  "hits": [
    {
      "path": "spec/components/tools/workspace-edit-substrate.md",
      "source": "spec",
      "kind": "component",
      "frontmatter_id": "workspace-edit-substrate",
      "score": 7.43,
      "snippet": "...post-edit syntactic <em>validation</em> for .rs files..."
    }
  ],
  "elapsed_ms": 12,
  "truncated": false
}
```

## Usage guidance for the agent

- For "where is X defined in code", prefer [[tools/lsp/rust-workspace-symbols]] — exact symbol lookup, faster and more precise.
- For "what does the spec say about X", prefer `text_search` with `source: spec`.
- For "is this concept discussed somewhere", `text_search` with `source: both`.

## See also

- [[tools/search/spec-search]] — structured query over metadata (kind/status/parent edges), not free-text
