```yaml
id: rust-code-actions
kind: tool
parent: components/rust-tools/lsp
order: 5
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/lsp
  - contracts/workspace-edit
code:
  - crates/oxidant-rust-tools/src/lsp_client.rs
tests:
  - crates/oxidant-rust-tools/tests/lsp_live.rs::code_actions_returns_list
status: active
responsibility: |
  Enumerate rust-analyzer code actions (quick-fixes, refactors, organise imports, implement missing members) for a range.
```

`category`: `ReadOnly` for enumeration; applying any action is `Mutating` and routes through the substrate.

## Schema

```json
{
  "type": "object",
  "required": ["file", "range"],
  "properties": {
    "file":  { "type": "string" },
    "range": {
      "type": "object",
      "properties": {
        "start": { "type": "object", "properties": { "line": { "type": "integer" }, "character": { "type": "integer" }}},
        "end":   { "type": "object", "properties": { "line": { "type": "integer" }, "character": { "type": "integer" }}}
      }
    },
    "kinds": { "type": "array", "items": { "type": "string" }, "description": "LSP CodeActionKind filter" }
  }
}
```

## Result

```json
{
  "actions": [
    {
      "title": "Implement missing members",
      "kind":  "quickfix",
      "edit":  { "changes": { ... } }
    }
  ]
}
```

Each action's `edit` is a `WorkspaceEdit` ready to feed to [[tools/edit/apply-edits]] or the substrate directly.

## Typical use cases

- `quickfix` actions tied to diagnostics — apply via [[flows/fix-diagnostic]].
- `refactor.extract` — extract function/variable.
- `source.organizeImports` — fix imports after a syn-add-use.
