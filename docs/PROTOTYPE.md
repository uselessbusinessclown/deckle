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
| Colour mode: two or three ink planes, per-plane codewords, calibration lattice | complete |

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
reserves the header field, but nothing encrypts yet. Anything sensitive must be encrypted
before it is handed to `deckle encode` — see [USE-CASES.md](USE-CASES.md), which says so
where people will actually read it.

**No QR symbology.** QR is Phase 1's compatibility layer for phone cameras and tiny
payloads. It is not started; the bootstrap page uses QR, but as a carrier for the
reference programs rather than through the symbology interface.

**Colour has no reference decoder.** `dkl_ref.py` reads black archives and *refuses*
colour pages with a message rather than misreading them. That is a deliberate pairing
with PLAN.md §18.8: colour is not rated for long-term storage, so the archival path
covers the archival mode. A colour archive needs Deckle. The `Symbology` trait of PLAN.md §6 is *described* but the code calls
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

**Colour**, same page and ECC, at 50% parity:

| Degradation | Survives to |
|---|---|
| Gaussian blur | 0.30 cell widths |
| Additive noise | 40 grey levels |
| Ink crosstalk (non-ideal inks) | 0.4 |
| Per-plane misregistration | 0.5 cell widths |
| Illumination gradient | 60% |
| Colour cast (lamp ageing) | 35% |
| Extra blue-channel noise | 40 grey levels |
| Yellow fade | 90% of density |
| An ink plane gone entirely | recovered at 50% parity |
| Scanned in greyscale | refused, by name |

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

**4.9 Colour reaches the full 3x ceiling, and the structure it needs is nearly free.**
PLAN.md §18.0 predicted 2.45-2.97x at equal cell size after the registration marks and
calibration lattice were paid for. Measured across A4, Letter and A6 at four cell sizes,
the ratio is **2.91 to 3.00**: the added structure costs about 0.5% of cells, which the
per-band codeword rounding mostly absorbs. The 3x ceiling itself is not negotiable - it
is what an RGB scanner's three measurements allow.

That is the software-loop number. The plan's *expected* 1.57-1.90x assumed colour would
need a coarser cell and a stronger ECC on real hardware, and nothing here refutes that:
the measured blur tolerance is 0.30 cell widths against black's 0.45, which is exactly
the kind of margin loss that forces a coarser cell. Colour's real gain is still a Phase 0
question.

**4.10 Four bugs in colour mode were all geometry, and all in the same class.** Each was
two pieces of code deriving the same physical quantity separately and disagreeing:

- The registration mark's centre, computed once in the encoder and once in the decoder,
  differed by half a cell through integer rounding. Now `reg_mark_centre` is the single
  definition both call.
- The per-plane transform was fitted straight to the mark positions, so it absorbed the
  local sync warp near the corners - and adding the warp again then double-counted it,
  drifting the sampling point by a third of a cell by the bottom of the page. The plane
  term has to be a *delta from the already-warped* black position.
- Sync marks overlapped the registration strips and were overprinted by the ink marks,
  poisoning both the warp field and the marks' own centroids.
- Calibration patches could land on a registration strip and overwrite a mark. Whether
  they did depended on how the 64-cell lattice lined up with the strips, so it appeared
  at 169 µm and not at 254 or 127.

The lesson is worth recording because it will recur: **anything both sides of the format
compute independently is a bug waiting to happen.** The cheap defence was the diagnostic
that found all four - dumping the encoder's cells and counting per-band, per-plane
cell-bit errors against them. Reasoning about the symptoms was slower and wrong twice.

**4.11 The degradation harness had to learn that black is a different ink.** Modelling a
dead magenta plane by lightening the green channel also erased the black corner markers,
because a rendered C+M+Y overprint and a K cell are the same pixels - as they are to a
real scanner. But carbon black does not fade when magenta does, so the renderer now
reports where K ink went and the fade model leaves it alone. Without that, the headline
plane-loss test failed for a reason that has nothing to do with the format.

**4.12 Sheets must carry equal shares, or the loss promise is false.** The encoder filled
sheets greedily, so the last one could be nearly empty — and then losing a *full* sheet
cost far more than its 1/N share of the blocks. `deckle estimate` was printing "any 1 of
3 sheets may be destroyed or missing" for an archive where that was not true. Blocks are
now spread evenly across the sheets, and a test asserts the arithmetic the promise rests
on: the fullest sheet's share must not exceed the parity fraction. This affected black
archives too; colour only made it show up, because tripling the capacity per sheet makes
the last-sheet remainder a much larger fraction of the whole.

**4.13 Two interleavers with a shared factor is a silent catastrophe.** Blocks stripe
across the parity groups with period `groups`, and across the ink planes with period
`planes`. At nine groups and three planes those aliased perfectly: *every parity group
landed wholly in one ink*, so losing that ink destroyed three groups outright instead of
costing every group a recoverable third. The symptom looked like a decoder problem —
"losing yellow is unrecoverable" — and was arithmetic. Keeping the two periods coprime
costs at most one extra group, and `plan` now does that explicitly.

The same class of thing bit the block-to-codeword mapping: filling plane 0's codewords
first put a small archive entirely in one ink. Blocks now round-robin across planes and
bands, which is what makes "an ink plane can fail" true for archives of any size rather
than only large ones.

**4.14 A "dead plane" test must be about conditioning, not readability.** Declaring an
ink dead substitutes a nominal column so the unmixing matrix stays invertible, and sends
that plane's blocks to parity. Set at a quarter of the strongest plane, it was condemning
a 30% faded ink that reads perfectly well and spending a third of the archive's parity to
do it. The bar belongs low - 3% - because a weak plane is better tried and failed: its
codewords then become erasures and reach parity by a route that recovers the ink if it
turns out to be legible.

**4.15 Cyan-and-magenta is a better trade than full colour more often than not.**
`--ink cm` gives exactly 2.00x against black, versus `cmy`'s 2.91-3.00x. What it buys for
that third: complete immunity to blue-channel noise and to yellow fade, and 0.6 against
0.4 tolerance of ink crosstalk, because there are two inks to separate rather than three.
Yellow is the least lightfast ink in most sets and is read in the noisiest channel a
scanner has, so it is the plane most likely to fail first in both storage and scanning.

**4.16 A photograph of a page is not a plane, and that was the whole barrier.** The first
real hardware test — a hand-held phone photo of a printed sheet, at 3.6 pixels per cell,
with a shadow across half the page — decoded nothing at all. Nothing about that was
resolution: a clean downsample to the same 3.6 px/cell reads with 0.02% cell errors.

Measuring the true local offset by correlating patches against the printed pattern showed
what was happening. The four corners fitted a homography perfectly, and the required
offset in the *middle* of the page ran from −7 to +7.5 cells, varying smoothly. Paper curl
and lens distortion together, and a homography maps a plane. The decoder searched ±1.1
cells around the prediction, so it locked onto whatever was there — the sampled cells were
statistically uncorrelated with the printed ones, 47% wrong, black and white confused
symmetrically.

The fix is to stop assuming the corner fit is sub-cell accurate everywhere. Sync marks are
now tracked outward from the corners by region growing, each one predicted from the
neighbours already found, so every search stays local however far the page has wandered.
Predicting by *extrapolating the gradient* rather than copying a neighbour matters: the
warp has a slope of over a cell per step in places, and a prediction that ignores it falls
behind, loses lock, and takes the rest of the page with it.

Three tuning results, each measured rather than guessed:

- **Reject aggressively.** A rejected sync mark leaves a hole its neighbours interpolate
  across, which is nearly free on a smooth field. An accepted *wrong* mark derails the
  growth. Raising the acceptance bar from 0.55 to 1.2 local standard deviations took the
  recovered blocks from 78 to 167 and the cell error from 13% to 1.5% — and improved fold
  tolerance at the same time.
- **A global surface fit is worse than local growth.** A quadratic in the page
  coordinates, with outlier rejection, seemed the principled model for curl plus lens
  distortion. It made things worse — 15% to 26% — because the real field has more
  structure than six coefficients, and the fit gets dragged by the region that is already
  wrong.
- **Close the sampling aperture.** At 6 px/cell the synthetic degradations cannot tell
  +/-0.13 cell from +/-0.08. At 3.6 px/cell on real paper they are not the same thing at
  all: the wider aperture reaches into the neighbouring cells. That single change was the
  difference between four blocks short and a byte-identical recovery.

The result: **a hand-held phone photograph of an A4 sheet at 254 um now decodes
byte-identically** — 144 blocks read directly, 27 rebuilt from cross-block parity, hash
verified, correctly reported as *recovered with difficulty*. The mean page warp was 7.5
cells; a flatbed reads 0.2.

This is the first thing in the project measured on real paper rather than in a software
loop, and it moved a design assumption: PLAN.md routes phone cameras to QR and the dense
grid to a scanner (§3). That is still the right default — this photo needed the whole
error-correction budget and the parity on top — but the grid is no longer scanner-only.

Fold tolerance did regress, 32 to 24 lines, as the cost of tracking large warps. That is
the right trade: 24 folds is far outside anything a stored sheet sees, and pages that are
not flat are the normal case for a camera.

**4.17 Second hardware test: colour on a good capture still fails, and the reason is
chromatic.** A `cm` sheet at 254 um, photographed flat, evenly lit, at 24.5 Mpx — better
than the first test in every respect — does not decode. The diagnosis is worth recording
because almost every plausible explanation turned out to be wrong:

- *Not resolution.* 5 pixels per cell, against 3.6 for the mono sheet that worked.
- *Not the warp.* It is large — 15 cells mean, which at 1.5% of the frame is textbook
  lens barrel distortion rather than curl — but the region growing tracks it. Forcing the
  warp to zero takes the cell error from 15% to 50%, so the tracking is doing real work.
- *Not dot gain.* A quarter of `cm` cells carry no ink, and the print measures 24% bare
  paper. The ink went down faithfully.
- *Not the sampling aperture.* Sweeping it moves the error by half a point.
- *Not structural black being confused with dark payload*, though that looked certain
  from the ideal print model. Measured, structural black reads (20,16,14) and the darkest
  payload (0,0,9): both effectively black in every channel, so `max(R,G,B)` separates
  nothing. It also costs robustness, since taking the brightest channel makes structure
  detection maximally sensitive to noise in the channel carrying no data.

What is left fits the evidence exactly. The residual error grows **radially**, 6.5% at the
page centre to 42% at a corner, and it is consistently worse for magenta (24%) than cyan
(15%) — different channels, different amounts. That is **lateral chromatic aberration**:
the lens focuses red and green at slightly different magnifications, so each ink plane
needs its own warp field. Deckle currently tracks one field, in luminance, and corrects
each plane with a bilinear fitted to four corner marks — which cannot represent a radial
per-channel difference.

**The fix is to track the warp per channel**, running the same region growing three times
over R, G and B. The sync marks are black, so they appear in every channel and are equally
findable in each; the result would absorb chromatic aberration exactly. That is the next
piece of decoder work, and it is what would make colour readable from a camera.

Two things this does not change. A flatbed has no chromatic aberration worth the name, so
colour on a scanner is unaffected. And the mono path is unaffected: the same photograph
pipeline reads a black sheet byte-identically.
