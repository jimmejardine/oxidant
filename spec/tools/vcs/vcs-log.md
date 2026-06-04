```yaml
---
id: vcs-log
kind: tool
parent: components/vcs/git-shellout
order: 4
implements:
  - contracts/tool
depends_on:
  - components/vcs/git-shellout
code:
  - crates/oxidant-vcs/src/tools/vcs_log.rs
status: active
responsibility: |
  Return recent commits for the active exploration's branch (or a revspec) as structured records.
---
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "revspec":     { "type": "string", "default": "HEAD" },
    "limit":       { "type": "integer", "default": 20, "maximum": 500 },
    "path":        { "type": "string", "description": "limit to commits touching this path" }
  }
}
```

## Result

```json
{
  "commits": [
    { "sha": "deadbee", "iso_date": "2026-05-21T15:11Z",
      "author": "James", "subject": "Wire RUSTC_WRAPPER=sccache" }
  ]
}
```

For full history including across all branches and with co-change analysis, prefer [[tools/timeline/code-changes]] / [[tools/timeline/spec-changes]].
