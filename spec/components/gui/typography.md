---
id: typography
kind: component
parent: overview
order: 9
implements: []
depends_on:
  - components/gui/viewport
code: []
status: active
responsibility: |
  Set the minimum readable text size across every panel and forbid the use of egui's `.small()` text helper for anything that needs to remain legible.
---

## Minimum font size

The egui default body size is the floor for any panel label. Panels MUST NOT call `RichText::small()` for de-emphasised or secondary text — `.small()` produces a noticeably sub-readable size on standard-density displays, especially when combined with the muted/faint colour band the theme exposes.

If a panel wants to de-emphasise text, the route is:
1. Apply a muted colour via `theme::muted_text()` or `theme::faint_text()`.
2. Keep the default font size.

Larger-than-default sizes are fine and encouraged for headings, the empty-transcript splash, and other elements that should draw the eye.

## Why a spec, not a lint

This is a project-level taste call rather than an enforceable contract, so the rule lives here. A future `clippy::disallowed_methods` config could mechanise the prohibition on `.small()`, but that is out of scope for the MVP.

## Heading sizes

Per-panel headings (the title row above the separator in each dock tab) use `RichText::strong()` at the default size. The transcript empty-state splash uses `.size(32.0)`. Beyond those two, panels SHOULD stick with the default size and rely on `strong()`, colour, and layout to establish hierarchy.
