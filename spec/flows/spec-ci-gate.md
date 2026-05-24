---
id: spec-ci-gate
kind: flow
parent: overview
order: 10
status: active
responsibility: |
  Dogfood loop: every push to main and every PR runs the oxidant binary's own `spec validate --strict` and `spec diff --strict` against the repo. Red badge = oxidant's tools just told us the spec has drifted from the code. Closes the loop opened by ADR 0008.
depends_on:
  - tools/spec/spec-validate
  - tools/spec/spec-diff
  - decisions/0008-spec-is-canonical
---

# Run the spec gate in CI

The CI half of the spec-driven design loop. The local half is [[flows/spec-first-edit]] (or [[flows/add-tool]] / [[flows/fix-diagnostic]]) — this flow is what catches the cases where the local half got skipped.

## Trigger

A `git push origin main` or a PR opened/synchronised against `main`. GitHub Actions fires `.github/workflows/spec.yml`.

## Steps

1. **Checkout + build.** Standard `actions/checkout@v4`, install the toolchain pinned by `rust-toolchain.toml`, restore the cargo cache via `Swatinem/rust-cache@v2`, then `cargo build -p oxidant --release`. The release build is what runs the gates — same binary code path users invoke locally, so behaviour can't drift between dev and CI.

2. **`oxidant spec validate --strict`.** Runs the [[tools/spec/spec-validate]] wrapper against the repo's `spec/` tree. Same checks the agent uses (frontmatter completeness, link integrity, length budgets, orphans, code-path existence, reachability, cycles, parse errors). `--strict` flips any warning into exit code 1.

3. **`oxidant spec diff --strict`.** Runs [[tools/spec/spec-diff]]. Same checks: trait-method drift for contract specs, missing `code:` paths for components and tools. `--strict` again gates the workflow.

4. **Badge updates.** GitHub renders the workflow status; the README's `[spec]` badge follows. Green = the tree is internally consistent and the code matches the contracts the spec claims. Red = one of the gates fired.

## Why the binary, not the test suite

We have integration tests in `crates/oxidant-spec-tools/tests/*.rs` that exercise the same validate/diff functions. Those run under [[tools/cargo/cargo-test]] in the `test` workflow. So why a separate `spec` workflow that builds the binary?

- **Dogfooding.** The CLI is the user-facing surface for these checks. Running the actual binary in CI proves the surface works end-to-end and stays exercised on every push.
- **Different exit semantics.** The integration tests assert *behaviour* of the underlying functions (with assertions like "returns N warnings", "counts contain ParseError"). The CLI gate enforces *policy*: zero warnings means green, anything else means red. The integration tests deliberately don't gate on warning count because the baseline shifts; the CLI gate does, because that's what a CI gate is for.
- **Visibility.** A dedicated badge means a contributor reading the README knows immediately whether the spec tree is healthy without parsing the test report.

## Fix loop when the badge goes red

1. **Run the failing gate locally.** `./target/release/oxidant spec validate --strict` (or `spec diff --strict`) reproduces the failure with the same output CI saw. The human-readable format names each warning with file:line.

2. **Identify which spec is wrong.** Most warnings cite a `spec_id` and a path. For `MissingCodePath`: either create the missing file or update the spec's `code:` list. For `Orphan` / `Reachability`: add a body `[[ref]]` from a reachable parent. For `UnresolvedRef`: fix the ref or remove it. For `MethodAdded` / `MethodRemoved` / `MethodSignatureChanged`: edit the contract trait block in the spec to match what the code declares, or change the code to match the spec — depending on which is authoritative for the change.

3. **Re-run the gate locally.** It should now exit 0.

4. **Push again.** Badge goes green.

## Failure modes that aren't drift

- **Sccache outage breaks the build step.** ADR 0005's runtime requirement was historically wired into CI too; we removed it from `test.yml` after a GitHub Actions Cache outage took the whole workflow down. The `spec` workflow only uses `rust-cache`. If a future contributor reintroduces sccache here, expect the same fragility.
- **Toolchain bump invalidates the cache.** First push after a `rust-toolchain.toml` change pays a full rebuild cost on the CI runner; the gate runs the same checks afterwards, just slower.
- **A genuine spec edit was made but the corresponding code edit is in a separate PR.** That's the system working as designed — don't split spec and code across PRs ([[decisions/0008-spec-is-canonical]]).

## See also

- [[tools/spec/spec-validate]] / [[tools/spec/spec-diff]] — what the gates actually run
- [[flows/spec-first-edit]] — the local discipline this gate enforces
- [[decisions/0008-spec-is-canonical]] — the source of the policy
