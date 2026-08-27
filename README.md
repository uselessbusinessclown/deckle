# Deckle

Backup data to paper, simple and efficient.

Deckle encodes files into print-ready pages of high-density binary cells and reads them
back from flatbed scans. A QR compatibility layer will cover phone cameras, tiny payloads,
and a bootstrap page that makes every archive recoverable **without Deckle** — from the
paper alone, plus a commodity QR reader and a Python interpreter.

## Status

Working prototype. Encode, render, degrade and decode all work end to end; a file
survives a whole sheet being destroyed and comes back byte-identical.

**An archive can be read back without Deckle.** Every archive ends with a bootstrap page
carrying, in plain language, what the format is and how to decode it — plus the complete
source of a reference decoder as ordinary QR symbols. Recovering it needs a QR reader and
a Python interpreter, and nothing else: the reference programs use the standard library
only, no NumPy, no Pillow, no pip.

That claim has been demonstrated, not just asserted. The QR symbols were read with Apple
Vision, which knows nothing about this project; the recovered programs matched the
SHA-256 printed beside them, were byte-identical to the files in `reference/`, and then
rebuilt a three-sheet archive with one sheet destroyed. CI repeats the check on every
commit with an independent QR decoder.

```bash
cargo build --release
./target/release/deckle estimate report.pdf --cell 169 --parity 0.3
./target/release/deckle encode   report.pdf --out pages --parity 0.6
./target/release/deckle decode   pages/page-*.png --out recovered
```

Or, from the paper alone, with nothing installed:

```bash
python3 dkl_ref.py scan-*.png -o recovered
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
crates/deckle-core/   layout engine, encoder, decoder, FEC, bootstrap page, harness
crates/deckle-cli/    the deckle binary
reference/            dkl_ref.py and dkl_fec.py - printed on every bootstrap page
docs/                 plan and prototype notes
```
