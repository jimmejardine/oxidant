```yaml
---
id: grep
kind: tool
parent: components/tools/fs
order: 4
implements:
  - contracts/tool
depends_on:
  - components/tools/fs
code:
  - crates/oxidant-tools/src/fs.rs
tests:
  - crates/oxidant-tools/src/fs.rs::grep_finds_matches_with_line_and_text
  - crates/oxidant-tools/src/fs.rs::grep_respects_path_glob
status: active
responsibility: |
  Stream-search a regex across the worktree (or a subset), returning line-anchored matches with context.
---
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["pattern"],
  "properties": {
    "pattern":     { "type": "string", "description": "regex (Rust regex crate syntax)" },
    "path_glob":   { "type": "string", "description": "limit to this glob" },
    "case_insensitive": { "type": "boolean", "default": false },
    "context":     { "type": "integer", "default": 0, "maximum": 5 },
    "limit":       { "type": "integer", "default": 200, "maximum": 5000 }
  }
}
```

## Result

```json
{
  "matches": [
    { "file": "src/foo.rs", "line": 42, "column": 13, "text": "fn handle(...)" }
  ],
  "truncated": false
}
```

## Engine

Backed by `grep-searcher` (the engine under ripgrep). Honours `.gitignore` by default. Binary files skipped.

## When to use vs text-search

`grep` is exact-match regex; [[tools/search/text-search]] is BM25-ranked, tokenised, and indexed. Use `grep` for known identifiers, `text_search` for conceptual queries.
