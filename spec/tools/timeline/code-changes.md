```yaml
---
id: code-changes
kind: tool
parent: components/spec-tools/timeline
order: 2
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/timeline
code:
  - crates/oxidant-spec-tools/src/tools/code_changes.rs
status: active
responsibility: |
  Return chronological change history for code files, optionally filtered by path / language / time window.
---
```

The code-tree counterpart to [[tools/timeline/spec-changes]]. Same git plumbing, different filter set.

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "path":      { "type": "string", "description": "file or directory path under worktree; omit for whole tree" },
    "lang":      { "type": "string", "description": "filter by file extension family (rs, toml, md, ...)" },
    "since":     { "type": "string" },
    "until":     { "type": "string" },
    "author":    { "type": "string" },
    "limit":     { "type": "integer", "default": 50, "maximum": 500 }
  },
  "additionalProperties": false
}
```

## Result shape

```json
{
  "commits": [
    {
      "sha": "deadbee",
      "iso_date": "2026-05-21T15:11:02Z",
      "author": "James",
      "subject": "Wire RUSTC_WRAPPER=sccache per exploration",
      "files_touched": [
        "crates/oxidant-rust-tools/src/cargo_runner.rs",
        "crates/oxidant-vcs/src/exploration.rs"
      ]
    }
  ]
}
```

## Pairing with spec timelines

Asking for "what changed in this area last week" is best answered by issuing both `spec_changes` and `code_changes` with the same `since`. The GUI does this in a single "Recent activity" panel; the agent can do likewise.

## See also

- [[tools/timeline/spec-changes]]
- [[components/spec-tools/timeline]]
