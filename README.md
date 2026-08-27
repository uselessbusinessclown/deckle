# Deckle

**Back up 61 kB to 246 kB net per sheet of A4 or Letter — two or three times that with
colour ink. Highly configurable, very robust backup to paper.**

Deckle encodes files into print-ready pages of dense black cells and reads them back from
a flatbed scan. Every archive ends with a bootstrap page carrying the format, the
procedure and a reference decoder as ordinary QR codes, so the paper can be read back
**without Deckle** — with a QR reader and a Python interpreter, and nothing else
installed.

## How much fits on a sheet

**These are net figures** — what your file actually gets. Reed–Solomon error correction,
the per-block index and CRC, the corner markers, the sync lattice and (in colour) the
calibration patches have all already been paid for. Nothing further is deducted.

| Cell size | A4 black | A4 `cm` | A4 `cmy` | Letter black | Letter `cm` | Letter `cmy` |
|---|---|---|---|---|---|---|
| 254 µm — conservative | **61 kB** | 121 kB | 182 kB | 59 kB | 117 kB | 176 kB |
| 212 µm | 88 kB | 176 kB | 264 kB | 85 kB | 169 kB | 254 kB |
| 169 µm — balanced | **139 kB** | 277 kB | 416 kB | 133 kB | 265 kB | 397 kB |
| 127 µm — aggressive | **246 kB** | 492 kB | 737 kB | 236 kB | 472 kB | 708 kB |

Two things move the real number, both in your favour or your control:

- **Compression comes first**, so text and configuration files usually need far fewer
  sheets than the table implies. `deckle estimate` measures it on your actual input.
- **Cross-block parity adds sheets**, it does not shrink these figures. At the default
  20% that is one extra sheet in six, and it is what lets a whole sheet be destroyed.

Start at 254 µm. It works on ordinary hardware and it is the default. The denser tiers
need a round trip on your own printer and scanner first — no hardware measurement has
been made yet, and `deckle simulate` cannot tell you what your printer's dot gain is.

### Three ink modes

| | bits/cell | capacity | durability |
|---|---|---|---|
| `--ink k` (default) | 1 | 1× | **archival.** Fused carbon black; no known fading mechanism on a hundred-year scale |
| `--ink cm` | 2 | 2× | **middle.** Cyan and magenta only |
| `--ink cmy` | 3 | 3× | **least.** Adds yellow |

`--ink cm` exists because **yellow is the weak link twice over**: it is the least
lightfast ink in almost every set, and it is read in the blue channel, which is the
noisiest a scanner has. Leaving it out costs a third of the colour gain and removes the
plane most likely to fail first — measured, it is untouched by any amount of
blue-channel noise or yellow fade, and tolerates half again as much ink crosstalk as
`cmy`.

Three bits is the ceiling, not four. An RGB scanner takes three measurements and cannot
separate a fourth ink plane, so black stays reserved for the corner markers, the sync
lattice and the descriptor — which is also what lets a colour page be located and
identified before any colour calibration exists.

Colour of any kind is **not rated for long-term storage**, is never the default, and has
to be asked for by name. If you cannot say when you will next re-print the archive, use
black.

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

| | black | `cm` | `cmy` |
|---|---|---|---|
| Gaussian blur | 0.45 cells | 0.30 cells | 0.30 cells |
| Additive noise | 40 grey levels | 40 | 40 |
| Dot gain / erosion | 0.3 / 0.4 cells | — | — |
| Illumination gradient | 60% | 60% | 60% |
| Fold lines | 32 | 8 | 8 |
| Speckle | 2000 blobs | 2000 | 2000 |
| Stain | 40% of page width | — | — |
| Missing full-width strip | 10% of page height | — | — |
| Rotation, mirroring | any angle, either handedness | same | same |
| A whole sheet destroyed | yes, at sufficient parity | same | same |
| Ink crosstalk (non-ideal inks) | — | 0.6 | 0.4 |
| Per-plane misregistration | — | 0.6 cells | 0.6 cells |
| Blue-channel noise | — | **immune** | degrades |
| Yellow faded away | — | **not used** | rebuilt from parity |
| An ink faded away | — | rebuilt at 100% parity | rebuilt at 50% parity |
| Scanned in greyscale | — | refused by name | refused by name |

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

Two situations, and they want different settings:

- **[Long-term archival](docs/USE-CASE-ARCHIVAL.md)** — the machines are gone, unpowered
  or unreadable. Black toner, enough parity to lose a whole sheet, and a bootstrap page so
  the paper outlives the software.
- **[Carrying data through a device inspection](docs/USE-CASE-TRANSPORT.md)** — the
  machine can be examined, imaged or kept, and the material still has to arrive. Colour
  for the capacity, because inks fading over decades does not matter to something that
  lives a week. Opaque, not hidden — and the encryption is doing all the work.

**[docs/USE-CASES.md](docs/USE-CASES.md)** is the shared reference: what to put on paper,
sizing, settings, storage, and the limits — including the one that matters most:
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

## Roadmap

- **Desktop apps for Windows, Linux and macOS.** The CLI is the engine; most people
  should never have to see it. A small app to drag a file in, choose a profile, see the
  sheet count, and print — and on the other side, scan and restore. The plan (§9.11)
  originally assumed a macOS-only SwiftUI app; targeting all three platforms reopens that
  choice in favour of a portable toolkit, and that decision is not yet made.
- **Phase 0 on real hardware** — printing the density ladder and measuring what a real
  printer and scanner can actually resolve. This is the largest unretired risk in the
  project: every capacity figure above comes from a software loop.
- **`docs/FORMAT.md`**, and then the real test of it: someone who did not write the
  encoder reimplementing the decoder from the specification alone.
- **QR as a first-class symbology**, for phone cameras and payloads small enough that a
  scanner is overkill.
- **Encryption**, so sensitive material does not have to be encrypted separately first.

**Contributions are very welcome** — especially Phase 0 measurements from real printers
and scanners, which need hardware rather than cleverness, and app work on any of the
three platforms. Open an issue or a pull request. The format, the plan and every measured
number are in `docs/`, and `cargo test` runs the whole software loop in under ten seconds.

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
