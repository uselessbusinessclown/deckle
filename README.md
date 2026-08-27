# Deckle

Backup data to paper, simple and efficient.

Deckle encodes files into print-ready pages of high-density binary cells and reads them
back from flatbed scans. A QR compatibility layer will cover phone cameras, tiny payloads,
and a bootstrap page that makes every archive recoverable **without Deckle** — from the
paper alone, plus a commodity QR reader and a Python interpreter.

## Status

Working prototype: Phase 0/1 of the plan. Encode, render, degrade and decode all work
end to end; a file survives a whole sheet being destroyed and comes back byte-identical.
The bootstrap page does not exist yet, so archives currently still need this tool to read
them — see [docs/PROTOTYPE.md](docs/PROTOTYPE.md) §3.

```bash
cargo build --release
./target/release/deckle estimate report.pdf --cell 169 --parity 0.3
./target/release/deckle encode   report.pdf --out pages --parity 0.6
./target/release/deckle decode   pages/page-*.png --out recovered
```

`deckle simulate FILE --degrade blur=0.3,folds=2,stain=0.1` runs the whole loop in memory
against a damage model, which is how the numbers below were measured.

Pure Rust, no `unsafe`, no platform-specific code. Builds and tested on macOS and Linux.

## Documentation

- **[docs/PLAN.md](docs/PLAN.md)** — planning and architecture: format, decisions with
  rationale, interfaces, capacity estimates, test matrix, risks, milestones. Read §0
  first; it challenges the project's core density premise with arithmetic and defines the
  Phase 0 go/no-go. §18 specifies **v1.1 "Chroma"**, an optional CMY colour mode that
  roughly doubles capacity per sheet and is explicitly *not* rated for archival storage.
- **[docs/PROTOTYPE.md](docs/PROTOTYPE.md)** — what the code does, how it deviates from
  the plan, six findings that amend the plan, and what it measurably survives.

## What it survives today

A4 at 254 µm, ECC Q, 20% parity. Each figure is the largest value at which the archive
still round-trips byte-identically.

| | |
|---|---|
| Gaussian blur | 0.45 cell widths |
| Additive noise | 40 grey levels |
| Dot gain / erosion | 0.3 / 0.4 cell widths |
| Illumination gradient | 60% corner to corner |
| Fold lines | 32 |
| Stain | 40% of page width |
| Missing full-width strip | 10% of page height |
| Rotation, mirroring | any angle, either handedness |
| A whole sheet destroyed | yes, at sufficient parity |

60,939 usable bytes per A4 sheet at 254 µm; 139,629 at 169 µm.

## Layout

```
crates/deckle-core/   layout engine, encoder, decoder, FEC, degradation harness
crates/deckle-cli/    the deckle binary
docs/                 plan and prototype notes
```
