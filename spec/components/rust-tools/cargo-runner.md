```yaml
---
id: cargo-runner
kind: component
parent: overview
order: 2
implements: []
depends_on: []
code:
  - crates/oxidant-rust-tools/src/cargo_runner.rs
tests:
  - crates/oxidant-rust-tools/src/cargo_runner.rs
  - crates/oxidant-rust-tools/tests/cargo_runner_live.rs
status: active
responsibility: |
  Spawn cargo/clippy/rustc/cargo-expand as subprocesses with --message-format=json, parse the streaming output via cargo_metadata, and return structured results.
---
```

Backs every cargo-* tool: [[tools/cargo/cargo-check]], [[tools/cargo/cargo-build]], [[tools/cargo/cargo-test]], [[tools/cargo/cargo-clippy]], [[tools/cargo/cargo-expand]], [[tools/cargo/cargo-tree]], [[tools/cargo/cargo-metadata]].

## Environment injection

Every cargo subprocess receives:
```
RUSTC_WRAPPER=sccache                 # unless user already has a non-sccache wrapper
CARGO_TARGET_DIR=<worktree>/target    # explicit per-worktree
CARGO_TERM_COLOR=never                # text output never has ANSI escapes
RUST_BACKTRACE=1                      # for test failures
```

See [[decisions/0005-no-shared-target-dir-use-sccache]].

## JSON-mode parsing

`--message-format=json` for `check|build|clippy`; `--format=json` (or `--message-format=json`) for `test` depending on harness. Output is line-delimited JSON; parse with `cargo_metadata::Message::parse_stream`.

Two outputs surface to the agent:
- The structured `Message[]` (compiler diagnostics, build artifacts, test events).
- A `summary` (count of errors/warnings, build success bool, test pass/fail tallies).

The agent never sees raw stderr scrollback.

## Test event extraction

`cargo test -- --format json --report-time -Z unstable-options` (or libtest's stable JSON when available) emits per-test events. Captured stdout/stderr is attached to the corresponding `TestFailure` so the model can read what the test printed.

## Cancellation

Long cargo invocations check `ToolContext::cancellation` between message-parse iterations. On cancel: SIGKILL the cargo subprocess and return a structured cancellation result (not an error).
