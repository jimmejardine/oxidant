```yaml
---
id: cargo-test
kind: tool
parent: components/rust-tools/cargo-runner
order: 3
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/cargo-runner
code:
  - crates/oxidant-rust-tools/src/cargo_runner.rs
tests:
  - crates/oxidant-rust-tools/tests/cargo_runner_live.rs::cargo_test_runs_a_passing_test
status: active
responsibility: |
  Run cargo test with structured per-test events, capturing stdout/stderr per failing test for diagnostic use.
---
```

`category`: `Mutating` (writes test binaries, may touch test fixtures on disk).

## Schema

```json
{
  "type": "object",
  "properties": {
    "package":   { "type": "string" },
    "filter":    { "type": "string", "description": "test name substring" },
    "features":  { "type": "array", "items": { "type": "string" } },
    "release":   { "type": "boolean", "default": false }
  }
}
```

## Result

```json
{
  "ok": false,
  "passed":  42,
  "failed":  2,
  "ignored": 1,
  "failures": [
    {
      "test":   "edit::tests::roundtrip_preserves_bytes",
      "package":"oxidant-tools",
      "stdout": "...",
      "stderr": "thread 'edit::tests::...' panicked at ...",
      "elapsed_ms": 13
    }
  ],
  "summary": { "elapsed_ms": 4231 }
}
```

## Output extraction

Uses libtest's stable JSON output when available, otherwise the unstable `--format json` with `-Z unstable-options`. Per-test stdout/stderr captured via the harness's `--report-time` and `--show-output` interaction.

## See also

- [[flows/fix-diagnostic]] — test failures are diagnostics for the agent loop
