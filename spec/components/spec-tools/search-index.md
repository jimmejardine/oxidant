```yaml
---
id: search-index
kind: component
parent: overview
order: 2
implements: []
depends_on:
  - components/spec-tools/index-db
code:
  - crates/oxidant-spec-tools/src/search_index.rs
tests:
  - crates/oxidant-spec-tools/src/search_index.rs
status: active
responsibility: |
  Maintain a Tantivy full-text index over spec markdown and Rust source for BM25-ranked search via tools/search/text-search and tools/search/spec-search.
---
```

A single Tantivy index at `<worktree>/.oxidant/search-index/`. Indexes both `spec/**/*.md` and `crates/**/*.rs` (and other configured code paths). One index, with a `source` field for filtering by `spec` vs `code`. Powers [[tools/search/text-search]] and [[tools/search/spec-search]] for the agent, plus the GUI's search box.

## Schema

| Field | Type | Stored | Indexed | Tokenized |
|---|---|---|---|---|
| `path` | text | yes | yes | basic |
| `source` | facet | yes | yes | — |
| `kind` | text | yes | yes | basic |
| `lang` | text | yes | yes | basic |
| `frontmatter_id` | text | yes | yes | basic |
| `content` | text | no | yes | code-aware tokenizer |
| `headings` | text | yes | yes | default |
| `mtime` | i64 | yes | yes | — |

`content` is not stored (saves disk); search hits return `path` + snippet (generated from content on read).

## Tokenizers

- For markdown specs: default Tantivy `en_stem` + lowercase + accent-fold.
- For Rust source: custom tokenizer that splits on Rust identifier boundaries (`_`, `::`, camelCase splits), preserves `unsafe`, `fn`, etc. as keywords, and emits both the original token and its lowercase form. So a search for `apply_edits` hits `apply_edits`, `applyEdits`, `ApplyEdits`.

## Build / update

- Initial build: walk both trees, parse markdown via [[components/spec-tools/frontmatter]] for the heading/kind/id fields, parse Rust files via `syn` for module-level structure (then index the raw text — `syn` is for metadata, not body tokenisation).
- Incremental update: same `notify` watcher as [[components/spec-tools/index-db]]. On file change, look up the document by `path`, delete, re-add.
- Commit on every batch end (debounce 200ms).

## Query API

```rust
struct SearchQuery {
    text: String,
    source: Option<Source>,         // None = both
    kind: Option<String>,           // specs only
    lang: Option<String>,           // code only
    limit: usize,
}

struct SearchHit {
    path: String,
    source: Source,
    score: f32,
    snippet_html: String,           // <em>-wrapped match positions
    frontmatter_id: Option<String>,
}

fn search(q: &SearchQuery) -> Vec<SearchHit>;
```

## Non-goals

- Not a semantic / vector search. Embeddings can be added later as a parallel index but BM25 is the right default — fast, deterministic, no model dependency, and well-understood ranking.
- Not real-time on the millisecond scale; the 200ms watcher debounce is intentional.
- Not a code-symbol search. For that, use rust-analyzer's `rust_workspace_symbols` ([[tools/lsp/rust-workspace-symbols]]) — different problem, different tool.
