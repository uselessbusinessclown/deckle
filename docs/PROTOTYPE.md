# Deckle prototype — what is built, what it does, and what it does not

This documents the first working implementation. It corresponds to **Phase 0/1** of
[PLAN.md](PLAN.md) plus the bootstrap page from Phase 2: the format, the layout engine,
the symbology, a complete software round trip with a CLI, and an archive that can be read
back **without Deckle**. It is a prototype, not v1.0, and §4 below is an honest
list of everything it does not yet do.

Pure Rust, no `unsafe`, no platform-gated code. The same crate builds and behaves
identically on macOS and Linux; CI runs the full suite on both.

## 1. Using it

```bash
cargo build --release
```

```bash
./target/release/deckle estimate report.pdf --cell 169 --ecc Q --parity 0.3
```

```bash
./target/release/deckle encode report.pdf --out pages --parity 0.6
```

Writes `pages/page-NNN.png` and `pages/archive.pdf`. Then, after printing and scanning
(or straight from the PNGs):

```bash
./target/release/deckle decode pages/page-*.png --out recovered
```

`deckle inspect page-001.png` reads a single page's descriptor and reports its
correction margin; add `--verbose` to dump the decoder's geometry when a page will not
read. `deckle simulate` runs render → degrade → decode entirely in memory:

```bash
./target/release/deckle simulate report.pdf --degrade blur=0.3,noise=20,folds=2,stain=0.1
```

Every subcommand takes `--json`.

## 2. What it implements

| Area | Status |
|---|---|
| Layout engine, capacity oracle, estimator | complete; the estimator *is* the layout engine and a test asserts they agree |
| Native raster: finders, sync lattice, banded interleave, whitening | complete |
| Reed–Solomon GF(2^8), errors + erasures | complete, verified at full capacity (2e + f = n − k) |
| Block framing with index and CRC-32C | complete |
| Cross-block erasure FEC, groups spanning pages | complete over GF(2^8); see §4 |
| Page descriptor, self-describing decode | complete; see §3 for the carrier deviation |
| Decoder: page location, orientation, warp, Sauvola, retry ladder | complete |
| Rendering: PNG and PDF at exact physical size | complete, image-mask path |
| Degradation harness and invariant tests | complete, 27 tests |
| Deflate compression with a skip-if-it-does-not-pay check | complete |
| Bootstrap page: procedure, parameters, and both reference programs as QR | complete |
| `dkl_ref.py` and `dkl_fec.py`, standard library only | complete |

**Verified end to end through a real PDF engine**: a file encoded to PDF, rasterised by
CoreGraphics at 600 dpi, and decoded back is byte-identical, with zero error-correction
capacity consumed.

**Verified end to end without Deckle**: the QR symbols on the bootstrap page were read
with Apple Vision - a decoder that knows nothing about this project - the recovered
`dkl_ref.py` and `dkl_fec.py` matched the SHA-256 values printed beside them and were
byte-identical to the files in `reference/`, and those recovered programs then rebuilt a
three-sheet archive with one sheet destroyed. That is the archival promise, demonstrated
rather than asserted. `tests/bootstrap.rs` runs the same check in CI using `rqrr`, an
independent pure-Rust QR decoder.

## 3. Deviations from the specification

Each is a deliberate prototype simplification, not a design change.

**The page descriptor is not a QR symbol.** PLAN.md fixed decision 3 puts the descriptor
in a standard QR so a commodity reader can extract it. This prototype carries the same
96-byte payload in a low-density black-only raster strip in the header band, protected by
RS(255,127) — 25% of the codeword correctable — with four corner markers giving it its own
homography. The payload layout is exactly what a QR would carry, so replacing the carrier
does not touch anything else. The bootstrap page prints the strip's geometry in words and
`dkl_ref.py` reads it, so the archival promise still holds; what is missing is the
convenience of a phone being able to read a data page's header directly.

**Cross-block FEC is GF(2^8), not GF(2^16).** A group is capped at 255 blocks (~46 KB), so
large documents split into several groups with pages striped across them. The interface is
unchanged by the upgrade; only `fec.rs` changes.

**Compression is deflate only.** zstd is the specified default (PLAN.md 9.5). Deflate is
the specified archival alternative and is what is implemented.

**No encryption.** `age` is specified (9.6); the descriptor carries the scheme byte and
reserves the header field, but nothing encrypts yet.

**No QR symbology, no colour mode.** QR is Phase 1's compatibility layer; colour is v1.1.
Neither is started. The `Symbology` trait of PLAN.md §6 is *described* but the code calls
the raster path directly — the trait should be introduced when the second symbology lands,
which is when it starts paying for itself.

**Vector render path not implemented.** Only the image-mask path (PLAN.md 4.1 default).
Ink inset (5.10) therefore cannot be expressed, since an image mask has no sub-sample
geometry — exactly as the plan predicted.

## 4. Findings that amend the plan

These came out of measurement, and the plan should be read with them.

**4.1 Interleaving must be banded, not page-wide.** PLAN.md §5.6 specifies one affine
permutation over the whole page. That is right for a thin burst — a fold disperses across
every codeword and costs each one a byte or two — and *wrong* for large-area loss:
dispersing a 6%-of-page hole puts about 120 bad bits in every codeword, past RS capacity,
so the entire page dies rather than a few codewords. Confining the permutation to 128-row
bands keeps the fold behaviour and turns a hole into a few wholly erased codewords, which
is what cross-block parity exists to rebuild.

| | page-wide interleave | 128-row bands |
|---|---|---|
| Missing full-width strip | 2% of page height | **10%** |
| Stain | 30% of page width | **40%** |
| Fold lines | 16 | **32** |
| Usable bytes per A4 sheet at 254 µm, ECC Q | 61,854 | 60,939 (−1.5%) |

**4.2 The sampling aperture should be tighter than 50%.** PLAN.md §5.8 proposes sampling
the central 50% of each cell. Measured against blur, ±0.13 cell beats ±0.18: at 0.4-cell
blur the worst-case correction margin drops from 66% to 44% of capacity. Narrower still
gains nothing and costs noise immunity.

**4.3 Dot gain really is asymmetric, and worse than erosion.** PLAN.md §5.10 argues that
ink spreading closes white cells and is therefore more damaging than thinning black ones.
Measured: dilation survives to 0.3 cell widths, erosion to 0.4+. The asymmetry is real,
which supports keeping the ink-inset parameter and the vector render path it requires.

**4.4 The finder must be at least 3 cells per unit.** Sized purely as a fraction of page
width (PLAN.md §5.2), a finder can come out 7 cells across on a small page, at which point
random payload cells reproduce its 1:1:3:1:1 signature often enough to mislead orientation
— 9 false candidates on one A6 page. A floor of 3 cells per unit reduced that to zero on
every page tested, for 2.4% of cells at A6 and 0.4% at A4.

**4.5 The bottom-right marker must be searched for, not predicted.** The plan locates the
fourth corner as TR + BL − TL. A homography does not preserve parallelograms, so under
keystone the true corner drifts — 150 px at 1% keystone. Searching a neighbourhood for the
darkest box, verified against the marker's white ring, handles it. Both tests must be
*relative* to local brightness: an absolute threshold rejects a good marker in a dim corner
under a 40% illumination gradient.

**4.6 Cell size must be quantised to device dots, not micrometres.** Four dots at 600 dpi
is 169.33 µm; no integer micrometre value is exactly four dots. The dot count is the
quantum and the physical size follows from it.

## 5. Measured behaviour

**Capacity** matches the plan's calculated tables. A4 at 254 µm, ECC Q: 726 × 970 cells,
2.02% structural overhead, 60,939 usable bytes per sheet against the plan's calculated
60.5 KiB. A4 at 169 µm: 1090 × 1456 cells, 139,629 bytes against a calculated 136.3 KiB.

**Degradation limits**, A4 at 254 µm, ECC Q, 20% parity, three sheets. Each figure is the
largest value at which the archive still round-trips byte-identically. Blur and dot gain
are in cell widths, so they mean the same thing at any density.

| Degradation | Survives to | Note |
|---|---|---|
| Gaussian blur | 0.45 cell widths | a 600 dpi scan of 254 µm cells is ~0.2 |
| Additive noise | 40 grey levels | |
| Dot gain (dilation) | 0.3 cell widths | |
| Erosion | 0.4 cell widths | |
| Illumination gradient | 60% corner to corner | |
| Speckle blobs | 2000 blobs | |
| Fold lines | 32 | |
| Stain | 40% of page width | |
| Missing full-width strip | 10% of page height | |
| Rotation | any angle, including exact quarter turns | |
| Mirroring | yes | caught by the descriptor retry |
| Scale error | 10% | |
| Perspective | 1.2% corner displacement | flatbeds have essentially none |
| Whole sheet destroyed | yes, at sufficient parity | asserted for every sheet position |

**Performance**, release build, one A4 sheet at 254 µm (704,220 cells): encode and render
~0.2 s, decode ~0.3 s.

## 6. What to build next

In the order that retires the most risk:

1. **The bootstrap page and `dkl_ref.py`.** Until these exist the project's central promise
   is unmet. This is also the honest test of the format spec, since the reference decoder
   must be writable from the spec alone.
2. **Real QR for the page descriptor**, replacing the raster strip. Restores the
   commodity-reader property and is the first user of the symbology interface.
3. **Phase 0 on real hardware.** Everything above is a software loop. The go/no-go
   measurement of PLAN.md §14 — the density ratio over QR on a real printer and scanner —
   has not been made, and it gates the native raster's existence.
