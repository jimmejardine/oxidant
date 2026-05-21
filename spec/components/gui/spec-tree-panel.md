---
id: spec-tree-panel
kind: component
parent: overview
order: 5
implements: []
depends_on:
  - components/spec-tools/index-db
  - components/spec-tools/graph
code:
  - crates/oxidant-gui/src/panels/spec_tree.rs
status: active
responsibility: |
  Left-docked tree view of spec/ organised by kind, with status/order ordering, recent-change badges, and validate-warning indicators.
---

## Layout

```
spec/
├── overview                  (active)
├── glossary                  (active)
├── components/
│   ├── core/
│   │   ├── agent-loop        (active) ●
│   │   └── ...
│   └── ...
├── contracts/                ⚠
└── ...
```

## Ordering

Within each directory:
1. Specs with explicit `order:` ascending.
2. Then alphabetical.

Directory groups follow the same convention.

## Badges

- `●` (filled dot): modified in the last 24h (from [[components/spec-tools/timeline]]).
- `⚠`: this subtree contains validate warnings (from [[components/spec-tools/validate]]).
- `(deprecated)`: status badge.

Tooltip on hover gives last-modified timestamp + commit subject.

## Interactions

- Click: open the spec as a centre tab (via [[components/gui/file-tabs]]).
- Right-click: context menu — Reveal in code (jumps to first `code:` path), Show inbound refs, Show outbound refs, Show drift.
- Drag onto a chat input: inserts the canonical ref as a `[[ref]]`.

## Backing query

The panel reads from the SQLite index on every refresh (every ~500ms, cheap). Live updates via the same `notify` watcher driving the index.
