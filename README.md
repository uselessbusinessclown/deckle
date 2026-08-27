# Deckle

**Back up 60 kB to 240 kB per sheet of A4 or Letter — three times that in colour.
Highly configurable, very robust backup to paper.**

Deckle encodes files into print-ready pages of dense black cells and reads them back from
a flatbed scan. Every archive ends with a bootstrap page carrying the format, the
procedure and a reference decoder as ordinary QR codes, so the paper can be read back
**without Deckle** — with a QR reader and a Python interpreter, and nothing else
installed.

## How much fits on a sheet

Usable bytes per sheet, before parity. Measured, not estimated — `deckle estimate` runs
the real layout engine on your real files.

| Cell size | A4, black | Letter, black | A4, colour | Letter, colour |
|---|---|---|---|---|
| 254 µm — conservative | **60 kB** | 59 kB | 182 kB | 176 kB |
| 212 µm | 87 kB | 84 kB | 261 kB | 253 kB |
| 169 µm — balanced | **139 kB** | 133 kB | 416 kB | 397 kB |
| 127 µm — aggressive | **246 kB** | 236 kB | 737 kB | 708 kB |

Start at 254 µm. It works on ordinary hardware and it is the default. The denser tiers
need a round trip on your own printer and scanner first — no hardware measurement has
been made yet, and `deckle simulate` cannot tell you what your printer's dot gain is.

Colour triples capacity and is **not rated for long-term storage**: colour inks fade
unevenly and yellow goes first. It is opt-in, by name, for a reason (`--ink cmy`).

## Configurable

Every knob has a safe default, and changing nothing gives a safe result.

```
--paper A4 | Letter | Legal | A3 | WxH    --cell 254        cell size in µm
--margin 12.7                             --dpi 600         render resolution
--ecc L | M | Q | H                       --parity 0.20     cross-block parity
--ink k | cmy                             --no-bootstrap    omit the documentation
--format png | pdf | both                 --landscape
```

## Robust

Every figure below is the largest value at which the archive still round-trips
byte-identically. A4 at 254 µm, ECC Q. Blur and dot gain are in cell widths, so they mean
the same thing at any density.

| | black | colour |
|---|---|---|
| Gaussian blur | 0.45 cells | 0.30 cells |
| Additive noise | 40 grey levels | 40 grey levels |
| Dot gain / erosion | 0.3 / 0.4 cells | — |
| Illumination gradient | 60% | 60% |
| Fold lines | 32 | 8 |
| Speckle | 2000 blobs | 2000 blobs |
| Stain | 40% of page width | — |
| Missing full-width strip | 10% of page height | — |
| Rotation, mirroring | any angle, either handedness | same |
| A whole sheet destroyed | yes, at sufficient parity | same |
| One ink faded away | — | yes, at 50% parity |
| Ink crosstalk / misregistration | — | 0.4 / 0.5 cells |

## What it's for

Which of your files still exist after the electronics are gone? Not a failed disk — that
is what backups are for — but a house fire that takes the NAS and the laptop, a flood, a
lightning strike, a severe geomagnetic storm, or twenty years passing with nothing left
that can read the medium.

Paper is not clever, but it is passive: no semiconductors to damage, no charge to lose,
no controller firmware to become unobtainable. Microfiche is denser and proven, but
creating and reading it needs specialised equipment. Deckle's bet is that the whole
recovery stack should be things anyone can buy, switch off and put in a drawer — **a
flatbed scanner, a phone with a QR app, and Python** — and that the decoder should travel
with the data, printed on the last sheet.

The trade is capacity. Tens to low hundreds of kilobytes per sheet: not a file server,
but enough for the password vault, the keys, the radio plan, the pump setpoints and the
deeds. Photos and disk images belong on offline drives; paper holds the root of trust.

**[docs/USE-CASES.md](docs/USE-CASES.md)** covers what to put on it, how to size and
configure it, how to store it, and the limits — including the one that matters most:
**Deckle does not encrypt anything yet, so encrypt before you print.**

## Using it

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
against a damage model, which is how the numbers above were measured.

Pure Rust, no `unsafe`, no platform-specific code. Builds and tested on macOS and Linux.

**An archive can be read back without Deckle**, and that has been demonstrated rather
than asserted: the QR symbols on the bootstrap page were read with Apple Vision, which
knows nothing about this project; the recovered programs matched the SHA-256 printed
beside them, were byte-identical to the files in `reference/`, and then rebuilt a
three-sheet archive with one sheet destroyed. CI repeats the check on every commit with
an independent QR decoder.

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

## Licence

MIT — see [LICENSE](LICENSE). The reference programs printed on every bootstrap page
carry the same licence in their own headers, so whoever recovers them decades from now
can see they are free to use.

```
crates/deckle-core/   layout engine, encoder, decoder, FEC, bootstrap page, harness
crates/deckle-cli/    the deckle binary
reference/            dkl_ref.py and dkl_fec.py - printed on every bootstrap page
docs/                 plan and prototype notes
```
