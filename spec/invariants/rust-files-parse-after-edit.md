---
id: rust-files-parse-after-edit
kind: invariant
order: 2
status: active
depends_on:
  - components/tools/workspace-edit-substrate
responsibility: |
  Every .rs file written by oxidant must parse cleanly with syn immediately after the write, or the entire WorkspaceEdit is rolled back.
---

The [[components/tools/workspace-edit-substrate]] runs `syn::parse_file` on every `.rs` file it has modified, in memory, before committing any writes to disk. If any post-edit file fails to parse:

1. No writes happen.
2. The WorkspaceEdit returns an error carrying the failing file's path and the `syn` error (with line/column).
3. Consumers ([[tools/edit/edit-string]], [[tools/edit/apply-edits]], LSP-driven refactors, syn transforms) see a structured failure and may retry or surface to the user.

This catches a large class of LLM mistakes immediately — at the edit step, before they propagate to `cargo_check` and confuse the diagnostic flow.

## Scope

- Applies only to `.rs` files. Other files (TOML, markdown, YAML) are written as-is; their validation is the responsibility of dedicated tools (`spec_validate` for markdown, etc.).
- Applies to all WorkspaceEdit producers, including LSP `rename` and syn transforms. Even rust-analyzer-produced edits go through this check.
- Does **not** check semantic validity (type errors). For that, run [[tools/cargo/cargo-check]].

## Why syn and not the full compiler

`syn` parsing is fast (sub-millisecond per file) and catches all syntactic errors. A semantic check via `cargo check` is much slower and is handled later in the [[flows/fix-diagnostic]] loop.
