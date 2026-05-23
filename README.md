# oxidant

[![lint](https://github.com/jimmejardine/oxidant/actions/workflows/lint.yml/badge.svg?branch=main)](https://github.com/jimmejardine/oxidant/actions/workflows/lint.yml)
[![test](https://github.com/jimmejardine/oxidant/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/jimmejardine/oxidant/actions/workflows/test.yml)

A Rust-native desktop code agent for working on Rust projects. Three things distinguish it from general-purpose agents: **first-class Rust tooling** (rust-analyzer, cargo, syn, clippy exposed as structured tools — not text-scraped), **spec-driven design** (the `spec/` tree is the source of truth; code realises spec, not the other way round), and **multi-exploration via git worktrees** (each side conversation is its own branch + worktree + rust-analyzer + `target/`). See [`spec/overview.md`](spec/overview.md) for the full picture.

**Status:** Pre-MVP. The `oxidant` binary is currently a scaffold that prints its version and a pointer to the spec tree. The interesting design lives in `spec/`.

## Environment

Everything oxidant expects on the host. Most items are needed for both development and runtime — the `Why` column says which.

| Tool | Install | Why |
|---|---|---|
| **Rust 1.95** | `rustup` picks it up from `rust-toolchain.toml` | Build + run. |
| **rust-analyzer** | `rustup component add rust-analyzer` | Runtime: oxidant spawns it for LSP-backed tools. Also needed by `lsp_live.rs` tests. |
| **git** | system package | Runtime: oxidant shells out for all VCS operations (see [`spec/decisions/0006-shell-out-to-git-cli.md`](spec/decisions/0006-shell-out-to-git-cli.md)). Also needed by `vcs_live.rs` tests. |
| **sccache** | `cargo install sccache --locked` | Runtime: oxidant sets `RUSTC_WRAPPER=sccache` on every cargo subprocess it spawns, so per-worktree `target/` directories can share compiles (see [`spec/decisions/0005-no-shared-target-dir-use-sccache.md`](spec/decisions/0005-no-shared-target-dir-use-sccache.md)). Hard-fail at launch if absent. |
| **An LLM provider** | local or commercial | Runtime: oxidant needs a chat completion endpoint. Either a local OpenAI-compatible server on `http://localhost:5000/v1` (Ollama, llama.cpp, textgen-webui — override with `OXIDANT_LOCAL_BASE_URL`), **or** an API key for a commercial provider (OpenAI / Anthropic — env-var names to be specified once provider wiring lands). |

## Build

```
cargo build --workspace
```

## Run

```
cargo run -p oxidant
```

Pre-MVP — the binary currently prints its version and a pointer to `spec/overview.md`. The agent loop, GUI, and worktree machinery exist as libraries (`crates/oxidant-core`, `crates/oxidant-gui`, `crates/oxidant-vcs`, …) but aren't wired into the CLI yet.

## Testing

**Use `cargo nextest run` for tests, not `cargo test`.** Nextest is faster, isolates tests in separate processes, and gives cleaner output. The one exception is doctests, which nextest does not run — use `cargo test --doc` for those.

One-time install:

```
cargo install cargo-nextest --locked
```

Common invocations:

```
cargo nextest run                                    # default suite (skips #[ignore])
cargo nextest run -p oxidant-spec-tools              # one crate
cargo nextest run --test validate_real_tree          # one integration target
cargo nextest run -E 'test(spec_read)'               # filter by test name
cargo nextest run --run-ignored=all                  # include #[ignore]'d tests
cargo test --doc                                     # doctests (nextest does not run these)
```

**Live tests heads-up.** Only two test suites are gated behind `#[ignore]`: `local_smoke.rs` (needs a local OpenAI-compatible server) and `cargo_runner_live.rs` (slow — spawns real `cargo` subprocesses). Other live suites — `lsp_live.rs` (spawns `rust-analyzer`) and `vcs_live.rs` (spawns `git`) — run on every `cargo nextest run`, so make sure those two binaries are on PATH before you test.

## Spec-driven design

`spec/` is canonical. For any change that crosses a contract, responsibility, or invariant, edit the relevant spec file first, then implement against it (see [`spec/decisions/0008-spec-is-canonical.md`](spec/decisions/0008-spec-is-canonical.md)). Drift is mechanically detectable via `spec_validate` and `spec_diff` — the same tools the agent uses on itself.

We dogfood: oxidant is built using oxidant. Friction you hit while editing a `tools/*` spec is usually the next thing worth fixing in the tool.
