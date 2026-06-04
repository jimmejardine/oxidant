```yaml
---
id: spec-changes
kind: tool
parent: components/spec-tools/timeline
order: 1
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/timeline
code:
  - crates/oxidant-spec-tools/src/tools/spec_changes.rs
status: active
responsibility: |
  Return chronological change history for one or more spec files, optionally filtered by kind / status / time window.
---
```

Surfaces git history for the spec tree, structured for the agent and the GUI's "recent activity" panel.

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "ref":       { "type": "string", "description": "canonical spec ref; omit for tree-wide" },
    "kind":      { "type": "string", "description": "filter by spec kind (tree-wide queries only)" },
    "status":    { "type": "string", "enum": ["draft", "active", "deprecated"] },
    "since":     { "type": "string", "description": "ISO-8601 or relative ('7 days ago')" },
    "until":     { "type": "string" },
    "author":    { "type": "string", "description": "git author name or email" },
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
      "sha": "a1b2c3d",
      "iso_date": "2026-05-21T14:02:11Z",
      "author": "James",
      "subject": "Adopt egui_dock for per-exploration window layout",
      "specs_touched": [
        "decisions/0003-egui-gui-over-tui",
        "components/gui/dock-layout"
      ]
    }
  ],
  "elapsed_ms": 35
}
```

## Common queries

- *"what spec changed last week?"* → `{ since: "7 days ago" }`
- *"history of this contract"* → `{ ref: "contracts/tool" }`
- *"all decisions added since v0.2"* → `{ kind: "decision", since: "<tag-date>" }`

## See also

- [[tools/timeline/code-changes]] — same shape, for `crates/**/*.rs`
- [[components/spec-tools/timeline]] — implementation including co-change detection (not yet exposed as a tool — surface added when there's a clear use case)
