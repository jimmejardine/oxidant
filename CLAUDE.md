# oxidant

A Rust-native desktop code agent for working on Rust projects. Three things distinguish it from general-purpose agents:

1. **First-class Rust tooling** — rust-analyzer, cargo, syn, clippy exposed as structured tools, not text-scraped from shell output.
2. **Spec-driven design** — `spec/` is the source of truth for design; code realises spec, not the other way round.
3. **Multi-exploration via git worktrees** — each side conversation is its own branch + worktree + rust-analyzer + `target/`.

See `spec/overview.md` for the full picture; every claim above lands somewhere in the spec tree.

## Spec-first working agreement

`spec/` is canonical (see `spec/decisions/0008-spec-is-canonical.md`). For any change that crosses a contract, responsibility, or invariant: **edit the relevant spec file first, then implement against it**. Bug fixes and internal refactors don't need spec edits.

When in doubt, run `spec_validate` and `spec_diff` (the tools the agent itself uses) to surface drift.

## Dogfooding

We are building this agent and using it on itself. Every iteration on oxidant's tooling, spec workflow, or worktree machinery is also an exercise of that same tooling on the oxidant codebase. Changes you make to a `tools/*` spec are typically driven by friction you just hit while making the previous change — capture that, don't smooth it over silently.

## Testing with nextest

We use [cargo-nextest](https://nexte.st) — faster runner, better isolation, cleaner output than `cargo test`.

One-time install:

```
cargo install cargo-nextest --locked
```

Common runs:

```
cargo nextest run                                    # everything fast (skips #[ignore])
cargo nextest run -p oxidant-spec-tools              # one crate
cargo nextest run --test validate_real_tree          # one integration target
cargo nextest run -E 'test(spec_read)'               # filter by test name
cargo nextest run --run-ignored=all                  # include #[ignore]'d live tests
                                                     # (cargo subprocess, rust-analyzer, local LLM server)
cargo test --doc                                     # nextest does not run doctests
```

Live tests are gated behind `#[ignore]` because they spawn real `cargo`/`rust-analyzer` subprocesses or expect a local OpenAI-compatible server. Run them deliberately, not on every iteration.
