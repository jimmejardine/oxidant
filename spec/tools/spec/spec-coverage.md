```yaml
id: spec-coverage
kind: tool
parent: components/spec-tools/coverage
order: 11
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/coverage
code:
  - crates/oxidant-spec-tools/src/tools/spec_coverage.rs
status: active
responsibility: |
  Model/CLI/Health-Check-facing wrapper over [[components/spec-tools/coverage]]: report Rust source files no spec transitively reaches.
```

`category`: `ReadOnly`.

## Schema

```json
{ "type": "object", "properties": {} }
```

No inputs — the analysis always runs over the whole workspace.

## Result

```json
{
  "seed_count": 64,
  "covered_count": 79,
  "count": 7,
  "uncovered": [
    { "file": "crates/oxidant/src/main.rs", "krate": "oxidant" }
  ],
  "missing_seeds": [],
  "notes": [ "File-level import-graph reachability …", "Heuristic: …" ]
}
```

`count` mirrors `uncovered.len()` so `--strict` / the Health Check can read a flat number.

## When this runs

- On demand from the CLI: `oxidant spec coverage [--strict] [--json]`.
- As the [[components/gui/health-check-panel]] `SpecCoverage` check.

## Deferred scope

Function-level coverage (a call graph rather than file-level import reachability) is a follow-up; see [[components/spec-tools/coverage]] "Limits".
