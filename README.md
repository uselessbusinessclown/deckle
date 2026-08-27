# Deckle

Backup data to paper, simple and efficient.

Deckle encodes files into print-ready pages of high-density binary cells and reads them
back from flatbed scans. A QR compatibility layer covers phone cameras, tiny payloads,
and a bootstrap page that makes every archive recoverable **without Deckle** — from the
paper alone, plus a commodity QR reader and a Python interpreter.

## Status

Planning. No implementation yet.

- **[docs/PLAN.md](docs/PLAN.md)** — planning and architecture document: format,
  decisions with rationale, interfaces, capacity estimates, test matrix, risks,
  milestones. Read §0 first; it challenges the project's core density premise with
  arithmetic and defines the Phase 0 go/no-go.

## Planned shape

- Portable Rust core with a C ABI, plus a CLI (`encode`, `decode`, `estimate`,
  `inspect`, `calibrate`, `simulate`).
- macOS app in Swift/SwiftUI wrapping the same core.
- Open bit-level format spec, frozen at v1.0, with a ~380-line Python reference decoder
  committed alongside it and printed on every archive's bootstrap page.
