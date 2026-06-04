```yaml
id: spec-for-file
kind: tool
parent: components/spec-tools/index-db
order: 4
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/index-db
code:
  - crates/oxidant-spec-tools/src/tools/spec_for_file.rs
tests:
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_for_file_finds_workspace_edit_substrate
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_for_file_with_windows_style_slashes
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_for_file_unknown_path_returns_empty
status: active
responsibility: |
  Reverse lookup: given a code file path, return the specs that reference it via their code: frontmatter.
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["path"],
  "properties": {
    "path": { "type": "string", "description": "code file path relative to worktree root" }
  }
}
```

## Result

```json
{
  "specs": [
    { "ref": "components/tools/workspace-edit-substrate", "kind": "component", "responsibility": "Apply atomic, span-precise multi-file edits..." },
    { "ref": "tools/edit/apply-edits", "kind": "tool", "responsibility": "Apply one or more span-precise edits..." }
  ]
}
```

## Use cases

- [[flows/fix-diagnostic]] step 2: locate the component spec for a diagnostic's file.
- Pre-edit context-gathering: "before I change this file, what does the spec say it should do?"
- GUI: clicking a code file in the spec tree panel reveals which specs claim it.

## Backed by

A single SQLite query against `spec_code_paths` in the index.
