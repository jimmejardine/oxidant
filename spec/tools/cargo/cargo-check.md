```yaml
id: cargo-check
kind: tool
parent: components/rust-tools/cargo-runner
order: 1
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/cargo-runner
code:
  - crates/oxidant-rust-tools/src/cargo_runner.rs
tests:
  - crates/oxidant-rust-tools/tests/cargo_runner_live.rs::cargo_check_ok_on_clean_project
  - crates/oxidant-rust-tools/tests/cargo_runner_live.rs::cargo_check_returns_diagnostics_on_broken_project
status: active
responsibility: |
  Run cargo check with --message-format=json and return structured CompilerMessages + a pass/fail summary.
```

`category`: `ReadOnly` (build artifacts are side-effects but the tool doesn't modify source).

## Schema

```json
{
  "type": "object",
  "properties": {
    "package":     { "type": "string" },
    "all_targets": { "type": "boolean", "default": false },
    "features":    { "type": "array", "items": { "type": "string" } },
    "no_default_features": { "type": "boolean", "default": false }
  }
}
```

## Result

```json
{
  "ok": false,
  "messages": [
    {
      "level": "error", "code": "E0308", "message": "mismatched types",
      "spans": [{ "file": "src/foo.rs", "start": {...}, "end": {...} }],
      "suggestion": null,
      "rendered": "error[E0308]: ..."
    }
  ],
  "summary": { "errors": 1, "warnings": 0, "elapsed_ms": 2140 }
}
```

## See also

- [[flows/fix-diagnostic]] — primary workflow consuming this tool
- [[tools/edit/apply-edits]] — to apply `suggestion.replacement` spans
- [[tools/cargo/cargo-clippy]] — broader lint set
