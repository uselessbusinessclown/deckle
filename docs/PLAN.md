# Deckle — Planning and Architecture

**Status:** draft v0.1, pre–Phase 0. Every capacity figure in this document is a
calculated estimate, not a measurement, and is marked as such.

**Scope of this document:** decisions, interfaces, formats, and sequencing, at a level
where a small team or a coding agent can execute without re-deriving them. It contains
no implementation code.

---

## 0. Executive summary and the one thing to read first

Deckle encodes files into print-ready pages of high-density binary cells, and decodes
them back from flatbed scans. A compatibility QR layer covers phone cameras, tiny
payloads, and the bootstrap page that makes an archive recoverable without the tool.

Working through the capacity arithmetic surfaced a finding that should shape Phase 0,
and it is stated up front because it challenges the project's premise:

> **The native raster's density advantage over QR is almost entirely a
> minimum-feature-size advantage, not a coding-overhead advantage.**

At the *same* feature size and comparable error-correction strength, a custom raster
beats tiled QR by roughly 1.15–1.4×. QR's quiet zones, finder patterns and format
information cost about 19% of page area; our fiducials and sync lattice cost about 2.5%.
That is a real but unremarkable win. The 5–8× win the project is premised on only
materialises if a purpose-built decoder can read cells at 0.127–0.17 mm where QR
practically requires 0.3–0.5 mm modules.

Consequences, both binding:

1. **Phase 0 gates the premise, not just the numbers.** Its go/no-go criterion is a
   density *ratio* measured on real hardware, not an absolute byte count (§14).
2. **If the ratio comes in below 3×, the correct response is to ship QR-only** with
   good tiling and strong cross-block FEC, and to drop the native raster. That is not a
   failure mode to be avoided; it is a cheaper product with the same user-facing promise.
   The architecture is built so this pivot costs weeks, not months: the symbology plugin
   interface (§6) is the seam, and QR is a first-class citizen behind it from day one.

Two other findings that change the design and are argued in full later:

- **Rendering 2.8 M vector rectangles per page produces ~8 MB of PDF per page.** A 1:1
  `ImageMask` produces ~350 KB with identical device output *when cell size is an integer
  number of device dots*. Fixed decision 7 is challenged on this evidence (§4.1).
- **The reference decoder can meet the ~400-line budget only if it is not required to
  perform cross-block erasure decoding.** It handles the all-pages-present path; a second,
  optional module on the bootstrap page handles loss (§5.11, §11.2 item 10).

**Colour (v1.1 "Chroma", §18)** is specified in full but held out of v1.0. The same
arithmetic discipline applies to it: adding CMY inks raises the ceiling to 3 bits per
cell, not 4 — an RGB scanner makes three measurements, so it cannot separate four ink
planes — and registration error plus a stronger ECC requirement bring the *expected*
gain to roughly **1.6–1.9×**, or **1.26×** net once colour's larger parity allowance is
counted. It is worth building as an increment on a shipped product. It is not worth
delaying v1.0 for, and it costs the twenty-year archival promise, which §18.8 turns into
policy rather than a caveat.

---

## 1. Product goal

Deckle turns arbitrary files into paper that can be read back by machine, decades later,
without Deckle.

**Encode.** One or more files → manifest → compress → optionally encrypt → chunk into
framed blocks → cross-block erasure code → symbol-encode → lay out on pages → render a
print-ready PDF or print directly. Multi-page, multi-file, deterministic.

**Configure.** Before printing, the user chooses pattern type, paper size, paper and
print type, reader type, density, symbol ECC level, and cross-block parity ratio, and
sees a live sheet count that is produced by the real layout engine, not an approximation.

**Decode.** Scanned pages — flatbed via ImageCaptureCore, image files, or phone photos
via Continuity Camera — → detect → reassemble → verify → original files, with a report of
exactly what was recovered and how much error-correction headroom was consumed.

**Survive.** Dust, speckle, folds, stains, a destroyed region, a missing page (given
parity), twenty years of storage, and the disappearance of Deckle itself. The binding
constraint: **every archive is recoverable from the paper alone, plus a commodity QR
reader and a Python interpreter.** Every decision below is checked against that sentence.

---

## 2. Symbology priority

| Rank | Symbology | Status in v1 | Role |
|---|---|---|---|
| 1 | **Native raster** | primary, main development effort | bulk payload, flatbed readers, highest density |
| 2 | **QR** | shipped in v1 | page descriptor, bootstrap page, phone-camera profile, tiny payloads, fallback bulk |
| 3 | Data Matrix / Aztec | not in v1 | available through the plugin interface later |

QR is not a token gesture. It is the compatibility floor, the bootstrap mechanism, and —
if Phase 0 goes badly — the product.

## 3. Non-goals for v1

Colour, which is specified separately as **v1.1 "Chroma"** (§18) and deliberately kept
out of v1.0 so the v1.0 format can freeze without waiting on it. Multi-level or tinted
cells remain out of scope entirely — ink levels between "absent" and "full" are precisely
what dot gain and fading destroy first, and no amount of calibration recovers them.
An iOS app (but the format must make one trivial: the page
descriptor is self-describing and the QR profile is phone-readable). Phone camera as a
reader for the native raster — phone readers are routed to QR. Anything cloud. Any
network access at all in the core.

---

## 4. Fixed decisions, and the two challenged with evidence

Decisions 1–6 and 8–10 from the brief are adopted as stated:

1. Native raster is primary; QR ships alongside through the same symbology interface.
2. Archival safeguard: open bit-level spec; reference decoder in Python (NumPy + Pillow
   at most) committed with the spec; every archive ends with a bootstrap page carrying a
   plain-text procedure and the decoder source as QR, with its SHA-256 printed beside it.
   *Adopted with one amendment — see §5.11.*
3. Per-page descriptor QR; the decoder never requires user-entered configuration.
4. Density provenance is printed on the page.
5. Pipeline order: compress → encrypt → chunk → cross-block erasure code → block framing
   → symbol encode.
6. The estimator *is* the layout engine in dry-run mode.
8. Portable core with a C ABI plus a CLI; macOS app in Swift/SwiftUI wraps it.
9. Binary cells only in v1.
10. QR via libqrencode or Nayuki qrcodegen, not `CIQRCodeGenerator`.

### 4.1 Challenge to fixed decision 7 (vector rendering)

**The decision as stated:** cells drawn as filled rectangles into a CoreGraphics PDF
context at exact physical size; never rasterize-then-scale.

**The principle is right. The mechanism does not scale.** A 0.127 mm page carries
2,820,273 cells, about 1.41 M of them black. Run-length merging along rows halves the
count for random data — roughly 705,000 rectangles per page. A PDF content stream spends
about 22 bytes per rectangle (`x y w h re` plus separators), so:

| Path | Content stream / page | After Flate | 10-page archive |
|---|---|---|---|
| Vector rects, RLE-merged | ~15.5 MB | ~8 MB | ~80 MB |
| 1:1 `ImageMask`, 1 sample per cell | 352 KB | ~350 KB (random data is incompressible) | ~3.5 MB |

CoreGraphics will also spend real time on 705 K fill operations per page, and some RIPs
degrade badly on content streams that large.

**The distinction that matters is not vector-vs-raster; it is whether resampling
occurs.** An `ImageMask` at exactly one sample per cell, placed at exact physical size,
with `/Interpolate false`, is resampled by the RIP at an *integer* ratio provided the
cell size is a whole number of device dots at the printer's native resolution — which is
exactly how profiles define cell size (3, 4, 5 or 6 dots). Under that condition the two
paths produce identical device output.

**Amended decision.** Render via 1:1 `ImageMask` by default, at the profile's nominal
dpi. Keep a vector-rectangle path behind `render.path = "vector"` for printers that
mishandle image masks. Neither path ever rasterizes at one resolution and scales to
another. Record the assumed dpi in the page descriptor and in the human-readable header.
**Phase 0 must print the same page both ways on both test printers and compare scans**;
if the image-mask path shows any device-level difference, the default flips.

*Reversibility:* high — it is one function behind the renderer interface, selected per
profile, and both paths must exist anyway for the test harness.

### 4.2 Challenge to the density premise (see §0)

Not a challenge to a stated decision, but to an unstated assumption. Carried into the
Phase 0 exit criteria (§14) as a measured ratio with a numeric threshold.

---

## 5. Native raster: the specification

All dimensions are physical. The renderer converts to device dots exactly once, at the
last step.

### 5.1 Cell geometry

Square cells of side `cell_um` micrometres, constrained to an integer number of device
dots at the profile's nominal dpi. At 600 dpi one dot is 42.33 µm, giving the ladder
127 / 169 / 212 / 254 µm for 3 / 4 / 5 / 6 dots. At 300 dpi the ladder starts at 254 µm.

**Non-square pixel aspect.** Some printers and most scanners have non-square native
sampling (e.g. 600 × 1200 dpi engines, 4800 × 9600 scanners). Deckle never emits
non-square cells. Instead the profile carries `render_dpi_x` and `render_dpi_y`; when
they differ, cell size is fixed by the *coarser* axis and the finer axis is oversampled
by an integer factor. On the decode side the scanner's reported x/y resolution is read
from the image metadata and the sampling grid is scaled anisotropically before the warp
model is fitted; if metadata is absent, the aspect is recovered from the measured
fiducial spacing, which is known to be square by construction.

### 5.2 Page structure

```
+--------------------------------------------------------------+  <- paper edge
|                        margin (12.7 mm default)               |
|  +--------------------------------------------------------+  |
|  |  HEADER BAND  (25 mm)                                   |  |
|  |  human-readable text .................   [descriptor QR]|  |
|  +--------------------------------------------------------+  |
|  +--------------------------------------------------------+  |
|  | [F]                                                 [F] |  |
|  |     . . . . . . . . . . . . . . . . . . . . . . . .     |  |
|  |     .   D A T A   C E L L   G R I D                .    |  |
|  |     .   with sync dots on a 16x16 lattice          .    |  |
|  |     . . . . . . . . . . . . . . . . . . . . . . . .     |  |
|  | [F]                                                 [f] |  |
|  +--------------------------------------------------------+  |
|                                                               |
+--------------------------------------------------------------+
```

- **Header band, 25 mm.** Human-readable text on the left (document name, page N of M,
  document ID prefix, creation date, cell size, ECC level, provenance mark, encryption
  status). Descriptor QR on the right: QR version 11, level M, 0.35 mm modules,
  22.8 mm square — comfortably inside the band, flatbed-trivial and phone-readable in
  good light.
- **Quiet frame, 2 cells** of white around the data grid.
- **Fiducials.** Three identical 24 × 24-cell concentric-square markers at top-left,
  top-right and bottom-left. A *different* 16 × 16 solid marker with a corner notch at
  bottom-right. Three-alike-plus-one-distinct resolves 90° rotation unambiguously.
- **Mirroring** is detected for free: a mirrored QR does not decode with a standard
  reader, so if the descriptor QR fails, the decoder retries the whole page mirrored
  before anything else.
- **Sync dots** on a 16 × 16-cell lattice, each a single cell forced to a known polarity
  (alternating by lattice parity, so a sync dot is never confusable with a stuck region).
  At 0.254 mm cells the lattice pitch is 4.06 mm; at 0.127 mm it is 2.03 mm.

### 5.3 Structural overhead budget

For A4 at 0.254 mm (726 × 970 cells = 704,220):

| Element | Cells | Share |
|---|---|---|
| Corner fiducials (3 × 576 + 256) | 1,984 | 0.28% |
| Sync lattice (1 in 256) | 2,751 | 0.39% |
| Quiet frame (2 cells) | 6,776 | 0.96% |
| **Total** | **11,511** | **1.63%** |

Budgeted at **2.5%** to leave room for spec growth, against the brief's ≤10% ceiling.
All capacity figures below use the 2.5% budget, not the 1.63% computed value.

The comparison worth internalising: QR's equivalent overhead — quiet zones, finders,
separators, timing, alignment, format and version information — is roughly **19%** of
tiled page area. That 17-point gap is the entire coding-overhead advantage, and it is
why §0 concludes the real advantage must come from feature size.

### 5.4 Block structure

The unit is a **Reed–Solomon codeword of 255 bytes** over GF(2^8).

```
 byte  0..2   block_index      u24 little-endian, unique within the document
 byte  3      flags            bit0 = parity block, bits1-7 reserved (zero)
 byte  4..7   crc32c           CRC-32C over bytes 0..3 and 8..k-1
 byte  8..k-1 payload
 byte  k..254 Reed-Solomon parity
```

Payload per block is `k − 8`. The 8-byte frame costs 3.1% at k = 255 and is what makes a
partial read useful: any single recovered codeword is self-identifying and
self-validating, independent of every other codeword and of the page descriptor.

CRC-32C (Castagnoli) rather than CRC-32: better minimum distance at these lengths and a
hardware instruction on every current CPU. It is a *misdecode* guard — RS already
detects most failures, but RS can silently mis-correct when errors exceed capacity, and
that is precisely the case where a silent wrong answer is most damaging.

### 5.5 Symbol ECC levels

| Level | RS(n, k) | Parity | Payload/block | Corrects | Erasures | Rate |
|---|---|---|---|---|---|---|
| L | (255, 239) | 16 | 231 B | 8 B | 16 | 0.906 |
| M | (255, 223) | 32 | 215 B | 16 B | 32 | 0.843 |
| **Q** (default) | (255, 191) | 64 | 183 B | 32 B | 64 | 0.718 |
| H | (255, 159) | 96 | 151 B | 48 B | 96 | 0.592 |

**Q is the default**, and this reconciles the brief's working expectations exactly: the
brief's ~60 KB / ~130 KB / ~240 KB per A4 are the level-Q numbers in §13. Blind profiles
use Q. Profiles verified by calibration ladder or round-trip may drop to M, which buys
about 17% more capacity.

**Erasure decoding is the payoff for soft information.** RS corrects `(n−k)/2` unknown
errors but `n−k` known erasures. The decoder's per-cell confidence lets it flag its
least-trusted cells as erasures, roughly doubling effective correction strength on the
retry ladder (§5.8). This is why LDPC was not needed to exploit soft decisions (§9.1).

### 5.6 Interleaving

Paper damage is two-dimensional and contiguous: a fold line, a coffee ring, a torn
corner. Undispersed, a 3-cell-wide fold across a 970-row page destroys 2,910 consecutive
cells and annihilates whichever few codewords they belong to.

**Two-stage interleave.**

1. **Affine address permutation.** For a page with `C` usable cell positions in raster
   order, `p' = (a·p + b) mod C`, with `a` coprime to `C`. Both `a` and `b` are in the
   page descriptor; `a` is chosen by the encoder from a fixed candidate list as the value
   maximising simulated burst dispersion for that page's geometry.
2. **Codeword assignment.** Codeword `c = p' mod N` where `N` is the codeword count on
   the page; bit position within the codeword is `p' div N`.

Worked example, A4 at 0.254 mm, level Q: `N = 686,614/8/255 ≈ 336` codewords. A vertical
fold of 2,910 damaged cells disperses to about 8.7 damaged *bits* per codeword. Because
successive hits on one codeword are one bit-index apart, they concentrate in one or two
bytes — so roughly 2 byte-errors per codeword against a capacity of 32. Comfortable.

**This must be verified, not assumed.** The affine permutation's dispersion depends on
number theory that varies with page geometry. The test harness (§15) treats burst
tolerance as a measured property across the whole configuration matrix, and encoder
startup asserts `gcd(a, C) == 1` and a minimum simulated dispersion score.

### 5.7 Cross-block erasure FEC

See §9.3 for the scheme choice. Layout: one parity group per document, up to 32,768 data
blocks per group; larger documents are split into multiple groups with pages striped
across them so that any single lost sheet costs each group an equal, small share.

**Parity groups span pages.** That is the whole point — the failure being defended
against is a lost or destroyed sheet. Systematic encoding means data blocks are printed
verbatim and parity blocks are appended on trailing sheets, so an undamaged archive never
touches the FEC decoder.

Default parity ratio: **20%**, i.e. one parity sheet per five data sheets, minimum one.
Stated to the user in the terms they care about: *"you can lose any 1 sheet in 6 and
still recover everything."*

### 5.8 Decoder pipeline

```
scan image
  -> 1. locate page          coarse threshold, connected components, find 4 fiducials
  -> 2. resolve orientation  3-alike + 1-distinct => rotation; QR failure => try mirror
  -> 3. global homography    4 fiducial centroids -> nominal grid space
  -> 4. read descriptor QR   ALL geometry and coding parameters come from here
  -> 5. local warp           displacement at each sync dot, bilinear between them
  -> 6. adaptive threshold   Sauvola, window = 4x cell pitch
  -> 7. sample cells         central 50% of cell area, bilinear at warped centre
  -> 8. per-cell confidence  (sample - local threshold) / local contrast
  -> 9. de-interleave        invert the affine permutation
  -> 10. RS decode per block retry ladder below
  -> 11. CRC32C verify       reject silent mis-corrections
  -> 12. emit blocks + report per-block correction counts, confidence histogram
```

**Warp model.** A global homography from four fiducials corrects skew and perspective but
not paper curl or scanner-bed nonlinearity. A full thin-plate spline over ~2,750 sync
points is a dense solve of that order and is not affordable. **Chosen: homography for
coarse registration, then a displacement field sampled at the sync dots and bilinearly
interpolated between them.** Linear in the number of sync dots, about 50 lines in the
reference decoder, and it corrects distortion at the 2–4 mm scale of the lattice, which
is the scale at which curl actually bends a page. Full TPS stays available as a
last-resort retry for pathological pages.

**Retry ladder,** applied per failed block, cheapest first:

1. Hard decisions, no erasures.
2. Flag the lowest-confidence 2% of that block's cells as erasures; re-decode.
3. Same at 5%, then 10%.
4. Exhaustively flip the single lowest-confidence cell (≤ 8 candidates).
5. Refit the local warp using only sync dots within 32 cells of the block's cells, and
   re-sample.
6. Re-threshold that region with a smaller Sauvola window (2× cell pitch).
7. Declare the block **erased** and hand it to cross-block FEC.

Failure reporting is per block, never per page: "page 4 recovered 331 of 336 blocks;
5 blocks reconstructed from parity; worst-block correction headroom 71%."

### 5.9 Scan input requirements

- **Lossless only.** TIFF (uncompressed or LZW) or PNG. JPEG is refused for cell sizes
  ≤ 0.17 mm and accepted with a loud warning above that.
- **8-bit greyscale.** Not 1-bit (throws away the soft information the retry ladder
  depends on), not colour (three times the data for no gain in a binary symbology).
- **Off:** descreening, unsharp mask, auto-exposure, auto-tone, auto-crop, dust removal.
  Scanner sharpening is the single most effective way to destroy a dense cell grid,
  because it creates ringing at exactly the spatial frequency of the cell pitch.
- **Resolution rule: at least 4 scan pixels per cell edge; 3 is the absolute floor.**

| Cell size | 600 dpi scan | 1200 dpi scan | Recommendation |
|---|---|---|---|
| 0.254 mm | 6.0 px | 12.0 px | 600 dpi |
| 0.212 mm | 5.0 px | 10.0 px | 600 dpi |
| 0.169 mm | 4.0 px | 8.0 px | 600 dpi |
| 0.127 mm | 3.0 px (floor) | 6.0 px | **1200 dpi** |

The acquisition layer sets these through ImageCaptureCore where the driver exposes them,
verifies them by reading back the resulting image's metadata and measured fiducial
spacing, and warns explicitly when a driver silently ignores a setting. It never
silently proceeds with a non-compliant scan.

### 5.10 Dot gain and ink inset

Toner and ink spread. Laser dot gain of 10–20% is normal, and it is *asymmetric* in the
way that matters here: black cells grow, and the white cells between them shrink. At
3-dot cells, an isolated white cell surrounded by black can close up entirely, which is
an unrecoverable structural error rather than a noisy one.

**Mitigation: `ink_inset_um`, a per-profile parameter.** Black cells are rendered inset
by that amount on each side so that dot gain restores them to nominal. Defaults: 0 for
inkjet on plain paper, 15 µm for laser at cell sizes ≤ 0.17 mm, 0 for laser above that.
Phase 0 measures the real value per printer via the calibration ladder, which prints an
inset sweep alongside the cell-size sweep.

Under the `ImageMask` render path (§4.1), inset cannot be expressed — an image mask has
no sub-sample geometry. This is the one place the two render paths genuinely differ, and
it is a second reason to keep the vector path alive: **inset requires vector rendering.**
Phase 0 must therefore answer both questions together — whether image-mask output is
device-identical, *and* whether inset is needed at the target cell size. If inset is
needed, the vector path becomes the default at small cell sizes despite its file size,
and the plan absorbs a ~8 MB/page PDF at 0.127 mm. Flagged as open question OQ-2.

### 5.11 Reference decoder and the bootstrap amendment

Fixed decision 2 requires a reference decoder under about 400 lines using at most NumPy
and Pillow. A realistic line budget for the native raster:

| Stage | Lines |
|---|---|
| Image load, greyscale, metadata | 10 |
| Fiducial location and orientation | 60 |
| Homography fit and apply | 40 |
| Sync-dot displacement field, bilinear warp | 50 |
| Sauvola threshold and cell sampling | 40 |
| De-interleave (affine inverse) | 15 |
| RS GF(2^8) decode — syndromes, Berlekamp–Massey, Chien, Forney | 120 |
| Block reassembly, CRC32C, ordering | 30 |
| Output, CLI | 15 |
| **Total** | **~380** |

It fits, but only because of an amendment that must be stated plainly:

> **The reference decoder does not perform cross-block erasure decoding.** It handles
> the case where every page is present and individually decodable — which is the
> overwhelmingly common recovery case — and reports which blocks it could not read.

Cross-block RS over GF(2^16) is another ~150 lines and would blow the budget. Rather
than weaken the FEC to fit the reference decoder, the bootstrap page carries **two**
QR-encoded modules with **separate printed SHA-256 hashes**:

- `dkl_ref.py` (~380 lines) — always needed.
- `dkl_fec.py` (~150 lines) — needed only if pages are missing or blocks unreadable.

The plain-text procedure on the bootstrap page says so in one sentence, so a person
recovering an intact archive never has to transcribe the second module. Decompression
is deliberately *not* in either module: `dkl_ref.py` emits the compressed stream and the
procedure names the one-line command to decompress it with any standard tool. That
keeps the compression choice (§9.5) off the reference decoder's critical path entirely.

---

## 6. Symbology plugin interface

The estimator, layout engine, renderer and reassembler must contain **no per-symbology
branches**. Everything they need is declared here. Expressed as a Rust trait; the C ABI
exposes a vtable of the same shape.

```rust
pub trait Symbology {
    fn id(&self) -> SymbologyId;              // stable u16, in the page descriptor
    fn name(&self) -> &'static str;

    /// What readers can read output at this density. Drives profile validation
    /// and the "reader type" configuration (§7).
    fn reader_requirements(&self, d: Density) -> ReaderClass;

    /// Feasible density range for this symbology on this medium.
    fn density_range(&self) -> (Density, Density);

    /// THE function the estimator and layout engine depend on. Pure, cheap,
    /// deterministic; no I/O, no allocation of page-sized buffers.
    fn plan_region(&self, area: PhysicalRect, d: Density, ecc: EccLevel)
        -> RegionPlan;

    /// Encode exactly `plan.payload_bytes` into a device-independent page
    /// description: a list of filled rectangles in millimetres, plus an optional
    /// 1-bit mask with its nominal dpi (§4.1).
    fn encode(&self, plan: &RegionPlan, payload: &[u8], hdr: &PageDescriptor)
        -> Result<PageDrawing>;

    /// Decode a scanned region. Returns per-unit confidence and correction
    /// counts so the reassembler can report margin without knowing the symbology.
    fn decode(&self, img: &GrayImage, hint: Option<&PageDescriptor>)
        -> Result<SymbolDecode>;
}

pub struct RegionPlan {
    pub payload_bytes: usize,      // usable bytes after ALL overheads
    pub raw_units: usize,          // cells or modules, for reporting
    pub structural_overhead: f32,  // fraction, for reporting
    pub unit_size_um: u32,         // cell/module size actually used
    pub grid: GridGeometry,        // symbology-private, opaque to callers
    pub block_size: usize,         // codeword payload; the chunker needs this
}

pub struct SymbolDecode {
    pub blocks: Vec<DecodedBlock>,      // each carries index, payload, CRC status
    pub erased: Vec<u32>,               // block indices present but unreadable
    pub confidence: ConfidenceReport,   // histogram + per-block headroom fraction
    pub corrections: CorrectionReport,  // symbols corrected / capacity, per block
    pub geometry_quality: f32,          // residual of the warp fit, 0..1
}
```

Two properties make the no-special-cases requirement hold:

- `plan_region` is the **only** capacity oracle. The estimator calls it; the layout
  engine calls it; nothing recomputes capacity from a formula.
- `block_size` flows *from* the symbology *to* the chunker, not the other way. The
  raster's 183-byte level-Q blocks and QR's tile payloads are both just numbers.

---

## 7. User configuration model

TOML on disk (`~/Library/Application Support/Deckle/profiles.toml`), JSON over the CLI
and the C ABI. Every field has a default; changing nothing yields a safe result.

```toml
[profile]
name            = "Balanced (blind)"
preferred       = true              # default for new encodes
created         = 2026-08-27

[symbology]
pattern         = "auto"            # auto | raster | qr
                                    # auto => raster if reader=flatbed, qr if phone

[paper]
size            = "A4"              # A4 | Letter | Legal | A3 | custom
width_mm        = 210.0             # custom only
height_mm       = 297.0             # custom only
orientation     = "portrait"        # portrait | landscape
margin_mm       = 12.7              # all four edges; 10.0 is safe on most lasers
header_mm       = 25.0              # reserved band; do not reduce below 24.0

[medium]
print_type      = "laser_plain"     # laser_plain | inkjet_plain | glossy | other
                                    # -> conservative default density + archival notes

[reader]
type            = "flatbed"         # flatbed | phone | either
scan_dpi        = 600               # flatbed only; validated against cell size (§5.9)

[density]
tier            = "balanced"        # conservative | balanced | aggressive | explicit
cell_um         = 169               # explicit only; must be an integer dot count
render_dpi      = 600
ink_inset_um    = 0                 # §5.10; measured by the calibration ladder

[ecc]
symbol_level    = "Q"               # L | M | Q | H  (§5.5)
parity_ratio    = 0.20              # cross-block FEC (§5.7)

[compression]
codec           = "zstd"            # zstd | deflate | none  (§9.5)
level           = 19

[encryption]
enabled         = false
scheme          = "age-scrypt"      # §9.6

[render]
path            = "imagemask"       # imagemask | vector  (§4.1)

[provenance]                        # written by the tool, not by hand (§8)
state           = "blind"           # blind | ladder_verified | roundtrip_verified
verified_on     = ""
printer         = ""
scanner         = ""
decode_rate     = 0.0
last_margin     = 0.0               # fraction of ECC capacity consumed
hardware_notes  = ""
```

**Plain-language notes** are attached to the two ECC controls in the GUI, because these
are the only settings where the user is trading sheets against a risk they cannot see:

- `symbol_level`: *"How much damage a single page survives — smudges, dust, a crease.
  Higher levels use more sheets."*
- `parity_ratio`: *"How many whole pages you can lose. At 20%, any 1 sheet in 6 can be
  destroyed or missing and the archive still restores completely."*

**Density tiers** map to cell sizes through the medium, so `balanced` means something
different on glossy inkjet than on plain laser:

| Tier | laser_plain | inkjet_plain | glossy | other |
|---|---|---|---|---|
| conservative | 254 µm | 254 µm | 212 µm | 254 µm |
| balanced | 169 µm | 212 µm | 169 µm | 254 µm |
| aggressive | 127 µm | 169 µm | 127 µm | 212 µm |

Every one of these numbers is a placeholder until Phase 0 replaces it with measurements.
Archival warnings ride on `print_type`: inkjet dye inks are called out as unsuitable for
multi-decade storage, laser toner and acid-free paper are recommended, and glossy stock
is flagged for scanner specular reflection.

---

## 8. Calibration and density provenance

**Calibration is optional and never blocks anything.** Any density may be chosen blind,
at any tier, with a one-line note and no confirmation dialog. The design intent is that
a user who never calibrates still gets a safe result, because new profiles start
conservative.

**Provenance states,** printed on every page and shown beside every estimate:

| State | Printed mark | Meaning |
|---|---|---|
| `blind` | `UNVERIFIED` | density chosen without measurement (default for new profiles) |
| `ladder_verified` | `LADDER 2026-08-27` | calibration ladder printed, scanned, analysed |
| `roundtrip_verified` | `ROUNDTRIP 2026-08-27` | real pages from this profile decoded successfully |

**Lightweight verification.** After printing, the app offers exactly one optional step:
*"Scan sheet 1 to confirm."* A successful decode with healthy margin upgrades the profile
to `roundtrip_verified` automatically, records the printer and scanner names, and stores
the measured margin. One scan, one click, no ladder.

**Decode margin** is reported on every decode, from `SymbolDecode.corrections`:

| Margin | Band | Shown as |
|---|---|---|
| < 40% of ECC capacity consumed | healthy | *"Comfortable. This density is working well."* |
| 40–75% | marginal | *"Working, but close to the limit. Consider a larger cell size."* |
| > 75%, or any block recovered via cross-block parity | recovered with difficulty | *"Recovered, but this density is too aggressive for this hardware."* |

Margin attaches to the profile, so the estimate for the *next* encode carries the
evidence from the last decode. When measured margin lands in the healthy band twice in a
row, the app offers to step the profile one tier denser — the only place Deckle
volunteers a density change, and it is always an offer.

**The calibration ladder** remains available on demand for both symbologies: a single
sheet printing a sweep of cell sizes (127 / 145 / 169 / 190 / 212 / 254 µm), each block
carrying known test data, crossed with an ink-inset sweep (0 / 10 / 15 / 20 µm). The
analyzer scans it and reports, per combination, the raw cell error rate and whether RS
at each level would have succeeded — from which it derives a recommended cell size with
a stated safety margin, and the measured `ink_inset_um`.

**Density drift** is the failure this system exists to catch: a profile verified on one
printer stays "verified" after the toner cartridge is replaced or the printer is. The
provenance record therefore stores printer and scanner identifiers, and the app warns
when the current hardware does not match the record — it does not silently downgrade the
profile, but it does say so beside the estimate.

---

## 9. Decisions

Each entry: **Options → Choice → Rationale → Reversibility.**

### 9.1 Symbol-level ECC scheme

**Options.** (a) Reed–Solomon over GF(2^8), byte-oriented, configurable parity.
(b) Golay(24,12). (c) LDPC with soft-decision belief propagation.

**Choice.** **RS over GF(2^8), n = 255, systematic**, with erasure decoding driven by
per-cell confidence (§5.5).

**Rationale.** Golay is disqualified on rate: 50% overhead to correct 3 bits in 24 is
far worse than RS at any level, and it is bit-oriented, so a burst inside one byte costs
three separate corrections instead of one. LDPC is the genuinely interesting rival — it
approaches capacity and it would exploit the soft per-cell information we already
compute, plausibly worth 15–25% more density. It loses on two counts that matter more
here than raw performance: there is no small, standard, re-implementable LDPC decoder
(iterative belief propagation with a specified code construction is far past 400 lines),
and burst errors still require the same interleaving, so its advantage is on the
*residual* random-error channel only. RS wins because a byte-symbol code treats a burst
inside a byte as a single error, its decoder is ~120 lines of classical algebra that any
competent engineer can re-derive from a textbook in 2050, and erasure decoding recovers
most of the soft-decision benefit LDPC was going to provide.

**Reversibility.** Moderate. The ECC scheme is identified by a byte in the page
descriptor and lives behind the block-framing layer, so a v2 LDPC mode can coexist. But
it changes the reference decoder and the format, so it is a format-version bump, not a
flag. LDPC is the top candidate for v2 and Phase 0 should record raw cell error rates in
a form that makes the LDPC gain computable offline.

### 9.2 Sync density, fiducial design, interleave depth

**Options.** Sync lattice at 8 × 8 (1.6% overhead), 16 × 16 (0.39%), or 32 × 32 (0.10%)
cells. Fiducials: QR-style three finders; four identical markers plus a separate
orientation cell; three-alike-plus-one-distinct. Interleave: none; row-block; affine
permutation.

**Choice.** **16 × 16 sync lattice** (configurable in the descriptor, so a profile may
tighten it). **Three-alike-plus-one-distinct** corner fiducials. **Affine address
permutation** for interleaving (§5.6).

**Rationale.** The sync lattice must resolve distortion at the scale paper actually
distorts — curl and cockling bend a sheet over 2–5 mm, not 0.5 mm. A 16 × 16 lattice is
2.0 mm at 0.127 mm cells and 4.1 mm at 0.254 mm, which brackets that range, at 0.39%
cost. 8 × 8 quadruples the cost to buy resolution below the physical scale of the
distortion. 32 × 32 is 8.1 mm at coarse cells, too coarse to catch curl. Three-alike-
plus-one-distinct gives unambiguous rotation with less area than four distinguishable
markers, and mirroring comes free from the descriptor QR's own chirality (§5.2). The
affine permutation costs one modular multiply per cell in the decoder — the cheapest
scheme that provably disperses a 2D burst, versus row-block interleaving which disperses
horizontal bursts well and vertical folds badly.

**Reversibility.** High for the lattice period and the interleave coefficients — both are
descriptor fields read at decode time. Low for the fiducial design, which is baked into
page location and therefore into the reference decoder.

### 9.3 Cross-block erasure FEC

**Options.** (a) RS over GF(2^8). (b) RS over GF(2^16), systematic, Cauchy matrix.
(c) RaptorQ (RFC 6330).

**Choice.** **Systematic RS over GF(2^16)**, groups of up to 32,768 data blocks,
spanning pages.

**Rationale.** GF(2^8) is eliminated by arithmetic: 255 total blocks per group at
183 bytes is a 46 KB group — smaller than a single page at any density above the
coarsest. Cross-*page* protection would be impossible. GF(2^16) gives 65,535-block
groups, about 12 MB, which covers essentially every realistic archive in one group and
makes the user-facing promise exact and simple.

RaptorQ is the strongest technical candidate and losing it hurts. It is linear-time,
near-optimal, and has a property nothing else offers: **parity can be generated later
without re-encoding**, so a user could print three more sheets next year and increase
their archive's resilience in place. It is rejected for v1 on archival grounds — RFC 6330
is a large specification, mature implementations are few, and a paper archive's whole
premise is that the algorithm can be re-implemented from the spec decades later.
Systematic RS also has the property that an undamaged archive never invokes the FEC
decoder at all, which is what lets the reference decoder skip it entirely (§5.11).

**Reversibility.** High. The FEC scheme is a descriptor field behind a trait with
`encode_group` / `decode_group`. RaptorQ is a clean v2 addition, and the incremental-
parity feature is a strong enough user story to justify revisiting it.

### 9.4 Cell sampling and thresholding

**Options.** Global Otsu; adaptive local (Sauvola / Niblack); per-cell matched filter.
Hard decisions vs soft decisions carried into decoding.

**Choice.** **Sauvola local thresholding**, window 4× cell pitch. Sample the **central
50%** of each cell's warped area with bilinear interpolation. **Hard decisions plus a
retained confidence value**, used as erasure flags on the retry ladder (§5.8).

**Rationale.** Global thresholding fails on the illumination gradient every flatbed
produces toward the bed edges, and on paper that is not uniformly white after twenty
years. Sauvola is the standard document-binarization answer, is about 15 lines with a
summed-area table, and adapts to local contrast, which is exactly what fading and
staining attack. Sampling only the central 50% avoids the cell edges, which is where dot
gain and scanner MTF do their damage — this alone is worth more than any thresholding
refinement. Full soft-decision decoding is rejected with LDPC (§9.1); erasure flagging
captures most of the benefit inside RS.

**Reversibility.** High. Entirely decoder-side; no format implications. It can be
improved for old archives after the fact, which is a genuinely valuable property.

### 9.5 Compression

**Options.** zstd; xz/LZMA2; deflate; none.

**Choice.** **zstd level 19 by default; deflate when the profile sets
`compression.codec = "deflate"`, which the archival preset does; xz rejected.**

**Rationale.** The tension is ratio versus twenty-year availability. zstd is
RFC 8878, is in the Linux kernel, and reached the Python standard library in 3.14 —
its long-term availability is now about as assured as deflate's, and it typically beats
deflate by 10–25% on mixed content, which is real sheets. Deflate's guarantee is
nevertheless absolute: it has been in every Python ever shipped and is decodable by
roughly every piece of software in existence. xz is rejected because its marginal ratio
gain over zstd (~10% at high levels) does not pay for a materially more complex format
and a smaller implementation base.

The stakes on this decision are lower than they look, and deliberately so: **the
reference decoder never decompresses.** It emits the framed, compressed stream and the
bootstrap page prints the one-line command to expand it (§5.11). A recoverer who cannot
find a zstd implementation still has their bytes; they have a data-format problem, not a
data-loss problem.

Content sniffing: the encoder compresses a 1 MB sample first and skips compression
entirely when the ratio is worse than 0.97, which catches already-compressed input
(JPEG, video, encrypted blobs) without wasting the user's time or claiming a sheet count
it cannot meet.

**Reversibility.** High — one byte in the document header, one pipeline stage.

### 9.6 Encryption

**Options.** age (scrypt passphrase recipient); AES-256-GCM with Argon2id, in a
container we define; none.

**Choice.** **age, passphrase mode.**

**Rationale.** Rolling our own container means specifying a KDF, its parameters, nonce
derivation, chunking for streaming, and authentication boundaries — five chances to make
a subtle, unfixable-in-print mistake. age is a small, reviewed, stable specification with
independent implementations in Go and Rust, an ASCII header, and a command-line tool
available on every platform; a recoverer in 2050 needs `age -d`, not our documentation.
Argon2id is more memory-hard than scrypt and that is a real advantage, but it does not
outweigh shipping a bespoke format on paper.

**On-page handling.** The human-readable header prints `ENCRYPTED: age-scrypt`. The age
header (~130 bytes, containing the scrypt salt and work factor) is carried in the
descriptor QR of **every** page, not just page 1 — losing the first sheet must not cost
the key-derivation parameters. Deckle never prints, stores, or transmits the passphrase,
and the printed page says in plain language that the archive is unrecoverable without it.

Order matters and follows fixed decision 5: encryption precedes chunking and FEC, so
parity protects ciphertext and a damaged page never blocks decryption of the rest.

**Reversibility.** High — a scheme byte in the document header.

### 9.7 QR tile encoding mode

**Options.** Byte mode; Base45 in alphanumeric mode.

**Choice.** **Byte mode for bulk QR payload; Base45 alphanumeric for the bootstrap page
only.**

**Rationale.** Base45 costs 3.1% (three alphanumeric characters at 5.5 bits each per two
bytes, versus 16 bits) and buys one thing: the content is readable as *text* by any
commodity QR reader, including a phone camera app that will happily show a person the
string. For the bootstrap page — whose entire purpose is being read by a stranger with
whatever tool they have — that property is the point, and 3.1% of one page is nothing.
For bulk payload it is 3.1% of every sheet, and any reader that can extract bytes can
extract bytes.

**Reversibility.** High — per-region, recorded in the descriptor.

### 9.8 QR detection strategy

**Options.** Apple Vision `VNDetectBarcodesRequest`; ZXing-cpp with grid-aware cropping;
both.

**Choice.** **ZXing-cpp in the core, with grid-aware cropping. Apple Vision as an
optional second-chance pass in the macOS app layer only.**

**Rationale.** The decisive constraint is not accuracy, it is testability: the software
loop (§15) must run in CI on Linux, and a macOS-only detector would make the QR path
untestable there. ZXing-cpp is portable, deterministic, and vendorable. Grid-aware
cropping — using the layout engine's own knowledge of where tiles were placed to crop
each one before detection — is worth more than detector quality anyway, because it turns
a hard multi-symbol detection problem into N easy single-symbol ones. Vision is
genuinely better on phone photos with perspective and glare, so the app tries it on
tiles ZXing rejects, and the outcome is recorded so we learn how often it matters.

**Reversibility.** High — a detector interface with two implementations.

### 9.9 Human-readable hex fallback for tiny secrets

**Options.** In scope for v1; out of scope.

**Choice.** **Out of scope for v1.**

**Rationale.** The use case — a 32-byte key, a seed phrase, a small certificate — is
already served by the QR compatibility symbology, which is machine-readable and needs no
transcription. A hex block's only advantage is being retypeable by hand, which is also
its defect: manual transcription is the least reliable link in any recovery chain, and
supporting it well means designing checksummed line groups, an unambiguous alphabet, and
a correction procedure — a small project of its own. Deferring it costs nothing because
nothing else depends on it.

**Reversibility.** Very high. It is a symbology plugin with a trivial `plan_region`, and
if added it should use Crockford Base32 with a per-line checksum, not raw hex.

### 9.10 Whether the descriptor QR carries a recovery aid

**Options.** Descriptor only; descriptor plus a per-page block map.

**Choice.** **Descriptor only, plus `first_block_index` and `block_count`.**

**Rationale.** A block map would be redundant: every block already carries its own index
and CRC (§5.4), so a recovered block is self-identifying without any page-level
metadata — which is a stronger property than a map provides, since it survives losing
the descriptor too. The two range fields cost 6 bytes and buy the one thing a map was for:
the ability to report *which* blocks a destroyed page held, so the user is told exactly
what parity must cover.

**Reversibility.** High — the descriptor is versioned and has reserved space.

### 9.11 Core language and stack

**Options.** Rust core with a C ABI; C core; Swift core; Python core.

**Choice.** **Rust core exposing a C ABI, plus a Rust CLI; macOS app in Swift/SwiftUI.**
This endorses fixed decision 8, with one architectural amendment below.

**Rationale.** Three things decide it. First, **the decoder parses untrusted input** —
arbitrary scanned images and attacker-influenceable payload — in code doing heavy pointer
arithmetic over image buffers and GF tables. That is the canonical memory-safety hazard,
and it rules out C for anything but a last resort. Second, **the dependency fit is
unusually good**: `reed-solomon-simd` and `reed-solomon-erasure` for coding, `image` for
I/O, `zstd`, and — decisively — `age`, whose reference-quality implementation `rage` is
Rust. Third, **CI must run on Linux**, which excludes a Swift core built on CoreGraphics
and excludes anything requiring macOS to test. Python is right for the *reference*
decoder and wrong for the product: if the shipped decoder were Python, the 400-line
constraint would either be violated or would cap the product's sophistication.

**The amendment: PDF generation must not cross the C ABI.** Fixed decision 7 puts PDF
output in CoreGraphics, which is macOS-only; if the core owned PDF generation, the
software loop could not render pages in CI. Instead **the core emits a device-independent
`PageDrawing`** — filled rectangles in millimetres plus an optional 1-bit mask and its
nominal dpi — and each host draws it: CoreGraphics in the app, a small pure-Rust PDF and
PNG writer in the CLI and the test harness. Both paths consume the identical
`PageDrawing`, so a rendering divergence is a testable defect rather than an untested
gap. This costs one extra data type and removes an entire class of "works on my Mac".

**Reversibility.** Low for the core language — this is the decision the whole repository
is built on. High for the amendment, which is an interface shape.

---

## 10. Estimator

The estimator is the layout engine in dry-run mode (fixed decision 6). It is not a
formula, it is not a second implementation, and there is a test that proves it (§15).

```
estimate(config, inputs) -> Estimate {
    input_bytes,
    compressed_bytes,        measured, or sampled+flagged (below)
    compression_ratio,
    usable_bytes_per_sheet,  from Symbology::plan_region
    data_sheets,
    parity_sheets,
    bootstrap_sheets,        1, or 2 if the reference modules overflow
    total_sheets,
    provenance,              blind | ladder_verified | roundtrip_verified + date
    last_decode_margin,      from the profile, if any
    warnings: [...]
}
```

**Compression is measured on the real input**, because a predicted ratio that is wrong
by 15% is a sheet count that is wrong by 15%, and the user finds out at the printer. For
inputs above 64 MB a **sampled fast path** compresses a stratified sample — the first
2 MB, the last 2 MB, and four 1 MB windows at deterministic offsets — extrapolates, and
labels the result **approximate** in the UI and with `"approximate": true` in the JSON.
The full measurement then runs in the background and the estimate updates in place.

**Live updates.** `plan_region` is pure and cheap, so any parameter change re-estimates
without recompressing; only a change to the compression settings or the input triggers
recompression.

**Impractical sheet counts.** Above 200 sheets, the estimator warns and suggests the
concrete alternatives — a denser profile with the sheets it would save, a lower parity
ratio with the protection it would cost, or a different medium — rather than a bare
"that's a lot". A 1 GB input is not a paper-backup problem and the tool should say so.

**Interface.** `deckle estimate --json` emits the struct verbatim. The GUI and the tests
consume the same call. There is no second code path.

---

## 11. Architecture

### 11.1 Data flow

```
 ENCODE
 ======
  file(s)
    |
    v
 [1] Ingest ------------------> manifest: names, sizes, mtimes, per-file SHA-256
    |                                        |
    v                                        v
 [2] Compress (zstd|deflate|none) <-- sniff 1 MB sample, skip if ratio > 0.97
    |
    v
 [3] Encrypt (age-scrypt, optional) -------> age header (~130 B) --+
    |                                                              |
    v                                                              |
 [4] Chunk into payloads of block_size, from Symbology::plan_region|
    |                                                              |
    v                                                              |
 [5] Cross-block FEC: systematic RS GF(2^16), groups spanning pages|
    |    data blocks unchanged; parity blocks appended             |
    v                                                              |
 [6] Frame each block: index | flags | CRC32C | payload            |
    |                                                              |
    v                                                              |
 [7] Layout engine ---- dry-run ----> [9] ESTIMATOR                |
    |    pages, regions, block ranges, interleave coefficients     |
    v                                                              |
 [8] Symbology::encode  (raster and/or QR)                         |
    |    + per-page descriptor QR <--------------------------------+
    v
   PageDrawing  (rects in mm + optional 1-bit mask @ nominal dpi)
    |                        |
    v                        v
 [10a] CoreGraphics      [10b] Rust PDF/PNG writer
       PDF / print             CLI, CI, test harness
    |
    v
 [11] Bootstrap page: plain-text procedure + dkl_ref.py + dkl_fec.py as QR
    |
    v
   PAPER


 DECODE
 ======
   PAPER
    |
    v
 [12] Acquisition: ImageCaptureCore | image files | Continuity Camera
    |    enforce lossless, 8-bit grey, >=4 px/cell, sharpening off
    v
 [13] Page detect, orientation, mirror resolution
    |
    v
 [14] Descriptor QR --> ALL geometry and coding parameters
    |
    v
 [15] Symbology::decode  (raster: warp -> threshold -> sample -> RS -> CRC)
    |    -> blocks + per-block confidence + correction counts
    v
 [16] Reassembler: dedupe across pages and rescans, order by block_index
    |    |
    |    +-- any blocks missing? --> [17] cross-block FEC decode
    v
 [18] Decrypt -> decompress -> verify SHA-256 -> write files
    |
    v
   RECOVERY REPORT: per page, per block, margin band, what was lost
```

The single most important structural property of this diagram: **the estimator [9] is a
branch off the layout engine [7], not a parallel path.** There is no second capacity
calculation anywhere in the system.

### 11.2 Components

| # | Component | Responsibility | Key interface |
|---|---|---|---|
| 1 | **Format spec** | bit-level document header, page descriptor, block frame, FEC group layout, bootstrap page | `docs/FORMAT.md`, frozen at v1.0 |
| 2 | **Config & profile store** | schema, defaults, validation, provenance records | TOML on disk, JSON at the boundary (§7) |
| 3 | **Symbology registry** | id → implementation; raster and QR plugins | `Symbology` trait (§6) |
| 4 | **Encoder pipeline** | stages [1]–[6]; deterministic, streaming where possible | `encode(config, inputs) -> Vec<Block>` |
| 5 | **Layout engine** | assign blocks to pages and regions; pick interleave coefficients; dry-run mode | `plan(config, n_bytes) -> LayoutPlan` |
| 6 | **Estimator** | §10; a thin wrapper over `plan` plus real compression | `estimate() -> Estimate` |
| 7 | **Renderer** | `PageDrawing` → PDF, print, preview; draws the human header and provenance mark | two backends, one input type (§9.11) |
| 8 | **Acquisition** | ImageCaptureCore, file import, Continuity Camera; deskew; lossless enforcement and warnings | `acquire() -> Vec<GrayImage + metadata>` |
| 9 | **Decoders** | raster and QR; structured output with confidence and corrections | `Symbology::decode` (§6) |
| 10 | **Reassembler** | dedupe across pages and rescans, erasure decode, hash verify, decrypt, decompress, partial-recovery report | `reassemble(blocks) -> Recovery` |
| 11 | **Bootstrap generator** | plain-text procedure, both Python modules as Base45 QR, printed SHA-256s | `bootstrap(doc) -> PageDrawing` |
| 12 | **CLI** | `encode`, `decode`, `estimate`, `inspect`, `calibrate`, `simulate` | JSON on stdout for every subcommand |
| 13 | **GUI** | config panel with live estimate → preview → print; scan-and-decode wizard; recovery and margin view | Swift/SwiftUI over the C ABI |
| 14 | **Calibration** | ladder generation and analysis; writes provenance | `calibrate` subcommand + GUI wrapper |
| 15 | **Test harness** | software loop, degradation models, config matrix, invariant tests | `cargo test` + `simulate`, runs in CI |

Component 10 deserves one note, because it is where partial recovery becomes a user-
visible promise: **the reassembler accepts the same page scanned many times.** A user who
gets a marginal result rescans at higher resolution, or flattens a fold and scans again,
and every attempt contributes blocks. Deduplication is by `block_index`, keeping the copy
with the better correction margin. This turns "recovery failed" into "recovery is
incomplete, here is exactly which pages to rescan" — the difference between a tool that
works and a tool that works when things have gone wrong.

---

## 12. Format specification and the C ABI

Byte tables below are the normative sketch; `docs/FORMAT.md` expands them and is the
artefact frozen at v1.0. All integers are little-endian.

### 12.1 Document header (first bytes of the framed stream, before compression is undone)

```
 off  size  field
   0     4  magic            "DKL1"
   4     2  format_version   u16, currently 0x0100
   6    16  doc_uuid         RFC 4122 v4
  22     8  created_unix     i64, UTC seconds
  30     1  compression      0=none 1=deflate 2=zstd
  31     1  encryption       0=none 1=age-scrypt
  32     1  fec_scheme       0=none 1=rs_gf16_systematic
  33     1  reserved
  34     4  fec_data_blocks  u32, per group
  35     4  fec_parity_blocks u32, per group
  39     8  plain_sha256_pre first 8 bytes of SHA-256 of the pre-compression stream
  47     2  manifest_len     u16
  49   ...  manifest         CBOR: [{name, size, mtime, sha256}, ...]
```

### 12.2 Page descriptor (payload of the descriptor QR, ~120–200 B, QR v11-M holds 271 B)

```
 off  size  field
   0     4  magic            "DKLP"
   4     2  format_version   u16
   6     2  symbology_id     u16  (1=raster, 2=qr)
   8    16  doc_uuid
  24     8  plain_sha256_pre
  32     2  page_index       u16, 0-based
  34     2  page_count       u16
  36     2  cell_um          u16
  38     2  grid_cols        u16
  40     2  grid_rows        u16
  42     2  grid_origin_x_um u16   offset of cell (0,0) from the data-area corner
  44     2  grid_origin_y_um u16
  46     1  sync_period      u8, cells (16)
  47     1  fiducial_spec    u8, 1 = three-alike-plus-notch
  48     4  interleave_a     u32
  52     4  interleave_b     u32
  56     1  rs_n             u8  (255)
  57     1  rs_k             u8  (239|223|191|159)
  58     1  block_payload    u8  (rs_k - 8)
  59     4  first_block_idx  u32
  63     2  block_count      u16
  65     1  compression      u8
  66     1  encryption       u8
  67     1  fec_scheme       u8
  68     1  provenance       u8  0=blind 1=ladder 2=roundtrip
  69     4  provenance_date  u32, days since 1970-01-01, 0 if blind
  73     2  render_dpi       u16
  75     2  ink_inset_um     u16
  77     1  age_header_len   u8
  78     8  reserved         zero in v1.0; allocated by colour mode in §18.9
  86   ...  age_header       present on EVERY page when encrypted (§9.6)
                             variable length, therefore always last
```

Every field a decoder needs is here. Nothing is user-entered at decode time.

### 12.3 Block frame

As §5.4: `index u24 | flags u8 | crc32c u32 | payload (k-8) | RS parity (255-k)`.

### 12.4 FEC group layout

Data blocks carry `flags.bit0 = 0` and indices `0 .. D-1`. Parity blocks carry
`flags.bit0 = 1` and indices `D .. D+P-1`. Group membership is implicit: block `i`
belongs to group `i / (D_g + P_g)`. Pages are striped across groups so that one lost
sheet costs each group an equal share.

### 12.5 Bootstrap page

Not a byte format — a human artefact. Contents, in order:

1. Title, document name, `doc_uuid`, SHA-256 of the plaintext, page count, creation date,
   Deckle version, format version.
2. A numbered plain-English decode procedure, ~15 lines, naming the compression and
   encryption tools by name.
3. A parameter summary in words: cell size, grid dimensions, RS parameters, interleave
   coefficients, sync period.
4. `dkl_ref.py` as Base45 QR tiles at 0.5 mm modules, with its SHA-256 printed in hex
   beneath, and the sentence *"These squares contain program source code. Any QR reader
   will show it as text."*
5. `dkl_fec.py` likewise, labelled *"only needed if pages are missing or damaged."*

Rendered conservatively at 0.5 mm modules (≈11.4 KiB per A4). Both modules compress to
roughly 6–7 KB combined after deflate and Base45, so one page normally suffices; the
generator spills to a second page rather than raising the density, because this page is
the last thing standing between the user and total loss.

### 12.6 C ABI sketch

Opaque handles, out-parameters, integer error codes, caller frees everything the library
returns. No callbacks into the caller on the hot path; progress is polled.

```c
typedef struct dkl_ctx      dkl_ctx;
typedef struct dkl_plan     dkl_plan;
typedef struct dkl_drawing  dkl_drawing;
typedef struct dkl_recovery dkl_recovery;

typedef enum {
    DKL_OK = 0, DKL_E_ARG, DKL_E_IO, DKL_E_FORMAT, DKL_E_UNSUPPORTED,
    DKL_E_DECODE_FAILED, DKL_E_INSUFFICIENT_PARITY, DKL_E_CRYPTO, DKL_E_INTERNAL
} dkl_status;

dkl_ctx*   dkl_ctx_new(void);
void       dkl_ctx_free(dkl_ctx*);
const char* dkl_last_error(dkl_ctx*);            /* borrowed, valid to next call */

/* Configuration and estimation. config_json follows the schema in section 7. */
dkl_status dkl_estimate(dkl_ctx*, const char* config_json,
                        const char* const* paths, size_t n_paths,
                        char** out_estimate_json);   /* caller: dkl_string_free */

/* Encoding. Produces a plan, then one drawing per page. */
dkl_status dkl_plan_create(dkl_ctx*, const char* config_json,
                           const char* const* paths, size_t n_paths,
                           dkl_plan** out_plan);
size_t     dkl_plan_page_count(const dkl_plan*);
dkl_status dkl_plan_render_page(dkl_ctx*, dkl_plan*, size_t page_index,
                                dkl_drawing** out_drawing);

/* Device-independent page description; the host draws it (section 9.11). */
typedef struct { float x_mm, y_mm, w_mm, h_mm; } dkl_rect;
size_t          dkl_drawing_rect_count(const dkl_drawing*);
const dkl_rect* dkl_drawing_rects(const dkl_drawing*);      /* borrowed */
int             dkl_drawing_has_mask(const dkl_drawing*);
dkl_status      dkl_drawing_mask(const dkl_drawing*, const uint8_t** bits,
                                 uint32_t* w, uint32_t* h, uint32_t* dpi,
                                 float* x_mm, float* y_mm);
void            dkl_drawing_free(dkl_drawing*);

/* Decoding. Feed pages in any order, any number of times (section 11.2, item 10). */
dkl_status dkl_recovery_new(dkl_ctx*, dkl_recovery** out);
dkl_status dkl_recovery_add_image(dkl_ctx*, dkl_recovery*,
                                  const uint8_t* gray8, uint32_t w, uint32_t h,
                                  uint32_t dpi_x, uint32_t dpi_y);
dkl_status dkl_recovery_status_json(dkl_ctx*, dkl_recovery*, char** out_json);
dkl_status dkl_recovery_finish(dkl_ctx*, dkl_recovery*, const char* passphrase,
                               const char* out_dir, char** out_report_json);
void       dkl_recovery_free(dkl_recovery*);

void       dkl_string_free(char*);
```

`dkl_recovery_status_json` is what lets the GUI say "4 of 7 pages read, 12 blocks
outstanding, rescan page 3" while the user is still at the scanner.

---

## 13. Capacity

> **Every number in this section is a calculation, not a measurement.** They assume the
> 2.5% structural overhead budget of §5.3, the block frame of §5.4, and the RS rates of
> §5.5. They assume nothing about whether a real printer and scanner can resolve the cell
> size in question — that is exactly what Phase 0 exists to determine. Treat the columns
> as *upper bounds conditional on the cell size working at all.*

Layout assumptions: margins 12.7 mm on all four edges, header band 25 mm.
Data areas: **A4** 184.6 × 246.6 mm, **Letter** 190.5 × 229.0 mm, **A3** 271.6 × 369.6 mm.

### 13.1 Native raster, 600 dpi

Cell sizes are 6, 5, 4 and 3 device dots at 600 dpi.

**A4**

| Cell | Grid | Cells | Raw | L | M | **Q (default)** | H |
|---|---|---|---|---|---|---|---|
| 254 µm | 726 × 970 | 704,220 | 88,027 B | 76.3 KiB | 71.0 KiB | **60.5 KiB** | 50.0 KiB |
| 212 µm | 871 × 1164 | 1,013,844 | 126,730 B | 109.8 KiB | 102.2 KiB | **87.1 KiB** | 71.9 KiB |
| 169 µm | 1090 × 1456 | 1,587,040 | 198,380 B | 171.8 KiB | 160.0 KiB | **136.3 KiB** | 112.6 KiB |
| 127 µm | 1453 × 1941 | 2,820,273 | 352,534 B | 305.4 KiB | 284.3 KiB | **242.2 KiB** | 200.1 KiB |

**Letter**

| Cell | Grid | Cells | Raw | L | M | **Q (default)** | H |
|---|---|---|---|---|---|---|---|
| 254 µm | 749 × 901 | 674,849 | 84,356 B | 73.1 KiB | 68.0 KiB | **58.0 KiB** | 47.9 KiB |
| 212 µm | 899 × 1081 | 971,819 | 121,477 B | 105.2 KiB | 98.0 KiB | **83.5 KiB** | 68.9 KiB |
| 169 µm | 1125 × 1352 | 1,521,000 | 190,125 B | 164.7 KiB | 153.3 KiB | **130.6 KiB** | 107.9 KiB |
| 127 µm | 1499 × 1803 | 2,702,697 | 337,837 B | 292.7 KiB | 272.5 KiB | **232.1 KiB** | 191.7 KiB |

**A3**

| Cell | Grid | Cells | Raw | L | M | **Q (default)** | H |
|---|---|---|---|---|---|---|---|
| 254 µm | 1069 × 1455 | 1,555,395 | 194,424 B | 168.4 KiB | 156.8 KiB | **133.6 KiB** | 110.3 KiB |
| 212 µm | 1282 × 1745 | 2,237,090 | 279,636 B | 242.2 KiB | 225.5 KiB | **192.1 KiB** | 158.7 KiB |
| 169 µm | 1604 × 2183 | 3,501,532 | 437,691 B | 379.2 KiB | 353.0 KiB | **300.7 KiB** | 248.4 KiB |
| 127 µm | 2138 × 2910 | 6,221,580 | 777,697 B | 673.7 KiB | 627.2 KiB | **534.3 KiB** | 441.4 KiB |

The level-Q column reproduces the brief's working expectation (≈60 / 130 / 240 KB per A4)
to within rounding. That correspondence is not a coincidence and it is why Q is the
default: the project's stated expectations were implicitly level-Q expectations.

### 13.2 Native raster, 300 dpi

At 300 dpi one device dot is 84.7 µm, so the cell ladder starts at 254 µm (3 dots).
**A 300 dpi printer cannot reach the interesting part of the design space at all** — the
densities that justify the native raster are unreachable. 300 dpi output is supported for
compatibility and should be treated as the QR-equivalent tier.

**A4 at 300 dpi**

| Cell | Dots | Grid | Raw | M | **Q** |
|---|---|---|---|---|---|
| 508 µm | 6 | 363 × 485 | 22,006 B | 17.7 KiB | **15.1 KiB** |
| 423 µm | 5 | 436 × 582 | 31,719 B | 25.6 KiB | **21.8 KiB** |
| 339 µm | 4 | 544 × 727 | 49,436 B | 39.9 KiB | **34.0 KiB** |
| 254 µm | 3 | 726 × 970 | 88,027 B | 71.0 KiB | **60.5 KiB** |

### 13.3 QR compatibility symbology

Estimated as (modules that fit) × (quiet-zone packing efficiency ≈ 0.87) ×
(0.074 data bytes per module² at level M — derived from the ISO/IEC 18004 capacity
tables, where large-version QR spends ~6% of modules on function patterns and level M
allocates ~37% of codewords to error correction). **These are the least certain figures
in this document and Phase 1 must replace them with values computed directly from the
standard's tables by the layout engine itself.**

**A4, QR level M**

| Module | Typical reader | Bytes/sheet | vs raster at the same feature size |
|---|---|---|---|
| 600 µm | phone camera, poor light | ~8.0 KiB | — |
| 500 µm | phone camera, good light | ~11.4 KiB | — |
| 400 µm | phone macro, or flatbed | ~17.9 KiB | — |
| 300 µm | flatbed 600 dpi | ~31.8 KiB | — |
| 254 µm | flatbed, optimistic | ~44.4 KiB | raster Q = 60.5 KiB → **1.36×** |

Letter is approximately 0.96× these values.

### 13.4 The ratio that matters

This is the table Phase 0 must fill in with measurements, and the one §0 turns on.

| Scenario | Raster | Best QR | Ratio |
|---|---|---|---|
| Optimistic: 127 µm cells decode reliably; QR needs 300 µm | 242.2 KiB | 31.8 KiB | **7.6×** |
| Expected: 169 µm cells decode; QR needs 300 µm | 136.3 KiB | 31.8 KiB | **4.3×** |
| Pessimistic: only 254 µm cells decode; QR manages 400 µm | 60.5 KiB | 17.9 KiB | **3.4×** |
| Failure: only 254 µm cells decode; QR manages 300 µm | 60.5 KiB | 31.8 KiB | **1.9×** |

The last row is the one to watch. If Phase 0 lands there, the native raster is buying a
factor of two for the cost of a bespoke symbology, a bespoke decoder, a bespoke spec, and
a reference implementation that must survive twenty years — and the plan should pivot to
QR-only. **The go/no-go threshold is 3×.**

---

## 14. Milestones

Durations are rough and assume one to two engineers.

### Phase 0 — spike (3–4 weeks). *Gates everything.*

Print a cell-size ladder (127 / 145 / 169 / 190 / 212 / 254 µm) crossed with an ink-inset
sweep (0 / 10 / 15 / 20 µm) for the native raster, and a module-size ladder
(254 / 300 / 400 / 500 / 600 µm) for QR. Two printers (one laser, one inkjet), two paper
types, one flatbed at 600 and 1200 dpi. Build a throwaway raster decoder good enough to
measure raw cell error rates on real scans. Also print one page via both render paths
(§4.1) and compare.

**Exit criteria — all four must be met to proceed with the native raster:**

1. A cell size exists at which raw cell error rate is below 0.5% on both printers.
2. The measured density ratio over the best working QR configuration is **≥ 3×** (§13.4).
3. Image-mask and vector render paths are shown either device-identical, or the vector
   path is shown necessary and its file sizes are accepted.
4. Whether ink inset is required at the chosen cell size, and its measured value.

**If criterion 2 fails, stop and re-plan as QR-only.** That re-plan reuses §6, §7, §8,
§10, §11 and §12 essentially unchanged; it discards §5 and roughly a third of §9.

### Phase 1 — format and round trip in software (5–7 weeks)

Format spec v0 (raster layout, block frame, page descriptor). Symbology trait with both
plugins. Encode and decode through the software loop. QR capacities computed from the
standard's tables, replacing §13.3. CLI with `encode`, `decode`, `estimate`, `inspect`,
`simulate`. **Exit:** a file round-trips through render → degrade → decode in CI, and
`estimate` provably equals the encoder across the config matrix.

### Phase 2 — resilience (4–5 weeks)

Cross-block RS over GF(2^16). Interleaving with measured burst tolerance. Multi-page,
multi-file, multi-group. Partial recovery and the recovery report. Bootstrap page
generator, and both Python reference modules written and tested against real output.
**Exit:** an archive with any one sheet in six destroyed, and a 15 mm hole punched
through every remaining sheet, recovers completely; and `dkl_ref.py` alone decodes an
undamaged archive.

### Phase 3 — macOS app, encode side (4–5 weeks)

Configuration panel with live estimate, preview, PDF export, direct print, printed
provenance mark. **Exit:** a non-technical user can encode and print without touching
the CLI, and printed output is byte-identical to CLI output.

### Phase 4 — macOS app, decode side (4–5 weeks)

ImageCaptureCore acquisition with settings enforcement, Continuity Camera, the
scan-and-decode wizard, margin reporting, automatic round-trip provenance upgrade.
**Exit:** scan-to-file with no manual configuration, on hardware not used in development.

### Phase 5 — hardening and freeze (3–4 weeks)

Calibration ladder polish and analyzer. Archival documentation. Reference decoder
finalized, line-counted, and verified by an engineer who did not write the encoder,
working only from `docs/FORMAT.md`. **Spec freeze at v1.0.**

That last exit criterion is the real test of the archival promise, and it should be
scheduled as an actual exercise with an actual person, not a checkbox.

---

## 15. Test matrix

### 15.1 Software loop

`render → degrade → decode`, entirely in memory, running on every commit in CI.

| Degradation | Parameters swept |
|---|---|
| Gaussian blur | σ = 0.3, 0.6, 1.0, 1.5 cell widths |
| Additive noise | σ = 2, 5, 10, 20 grey levels |
| Rotation | ±0.2°, ±1°, ±3°, ±10° |
| Skew / shear | 0.5%, 1%, 2% |
| Scale error | ±0.5%, ±1%, ±2% |
| Perspective | corner displacement 0.5%, 1%, 2% of page width |
| JPEG artifacts | quality 95, 85, 70 |
| Dot gain (dilation) | 5%, 10%, 20% of cell width |
| Erosion | 5%, 10% |
| Illumination gradient | 10%, 25%, 40% corner-to-corner |
| Speckle / blobs | 10, 100, 1000 blobs of 0.5–3 mm |
| Fold lines | 1, 2, 4 lines, 1–4 cells wide, H and V and diagonal |
| Stain | circular, 10–40 mm, 50% and 100% opacity |
| Torn corner | 10%, 25% of page area removed |
| Missing strip | 5 mm and 20 mm full-width |
| Missing pages | 1 page, exactly the parity limit, one beyond the limit |
| Inversion | full-page polarity flip |
| Mirroring | horizontal flip |
| Rotation by 90/180/270 | all three |

Crossed with the configuration matrix: {A4, Letter} × {254, 212, 169, 127 µm} ×
{L, M, Q, H} × {raster, QR} × {0%, 20%, 40% parity}. The full cross product is large;
CI runs a fixed pseudo-random 200-cell sample per commit with a seed derived from the
commit hash, and the complete matrix nightly.

### 15.2 Invariant tests

These are the tests that catch the failures that would be worst:

- **Estimator equals encoder.** For every configuration in the matrix,
  `estimate().total_sheets == encode().pages.len()`. This is the test that keeps fixed
  decision 6 true as the code changes.
- **End-to-end hash.** Every round trip verifies the plaintext SHA-256, so a silent FEC
  or RS mis-correction is a test failure rather than a corrupt archive.
- **Bootstrap page reads with a generic decoder.** The bootstrap QR tiles are decoded by
  ZXing with no Deckle-specific knowledge, the recovered source is hashed and compared to
  the printed SHA-256, and the recovered `dkl_ref.py` is *executed* against a rendered
  archive. This is the archival promise, tested.
- **Reference decoder parity.** `dkl_ref.py` and the Rust decoder produce identical block
  output for every page in the matrix.
- **Property-based FEC.** Random block sets with random erasure patterns at and below the
  parity limit always reconstruct; beyond the limit, the failure is always *reported*,
  never silent.
- **Burst dispersion.** For every page geometry, the affine interleave achieves a measured
  minimum dispersion score; asserted at encode time as well as in tests.
- **Determinism.** The same input and config produce byte-identical PDFs.
- **Render path equivalence.** Image-mask and vector renderings of the same page rasterize
  identically at the profile's nominal dpi.

### 15.3 Hardware loop

Not automated, but documented as a protocol so results are comparable across people and
across months: fixed printer settings (no scaling, no toner saving, no draft mode),
fixed scanner settings (§5.9), a recorded hardware inventory, a fixed test corpus, and a
results file committed to the repository. Run at the end of each phase from Phase 2 on,
and whenever a default density changes.

---

## 16. Risk register

| # | Risk | Severity | Mitigation |
|---|---|---|---|
| R1 | **Raster decoder cannot handle real scanner blur, MTF and local distortion at useful densities** | critical | Phase 0 prototype on real scans is the first thing built; go/no-go before any product work. §13.4 threshold. |
| R2 | **The density advantage is smaller than assumed and the raster is not worth its complexity** | critical | Stated in §0; measured in Phase 0 as a ratio; QR-only pivot pre-planned and cheap because of §6. |
| R3 | Dot gain closes white cells at 3-dot sizes | high | `ink_inset_um` (§5.10), measured by the ladder; inset sweep is part of Phase 0. |
| R4 | Scanner auto-sharpening or JPEG destroys cells and the user never knows | high | Acquisition enforces settings, reads back metadata, measures fiducial spacing, and refuses JPEG below 170 µm (§5.9). |
| R5 | Spec or decoder lost; archives unreadable in 2050 | high | Open bit-level spec, bootstrap page, reference decoder verified by an outsider from the spec alone (Phase 5 exit). |
| R6 | FEC or RS bug silently corrupts data | high | End-to-end SHA-256 on every round trip; CRC32C per block as a mis-correction guard; property-based FEC tests (§15.2). |
| R7 | Estimator drifts from the encoder; users print the wrong number of sheets | medium | Estimator *is* the layout engine (§10) and the equality is asserted across the matrix (§15.2). |
| R8 | User mixes pages from different documents | medium | `doc_uuid` and `plain_sha256_pre` in every page descriptor; the reassembler rejects foreign pages by UUID and says which document they belong to. |
| R9 | Density drift when hardware changes under a verified profile | medium | Printer and scanner identifiers in the provenance record; explicit warning on mismatch (§8). |
| R10 | Print spooler retains plaintext of an encrypted archive | medium | Encryption happens before rendering, so the spooler only ever sees ciphertext cells. Documented, and the GUI says so where the passphrase is entered. |
| R11 | Scope growth from carrying two symbologies | medium | The plugin interface (§6) is the discipline: no per-symbology branches outside a plugin is a reviewable rule. |
| R12 | PDF size makes the vector path unusable at high density | medium | §4.1; image-mask default; measured in Phase 0. |
| R13 | Reference decoder exceeds its line budget as the format grows | medium | Budget tracked in CI as a hard limit; the FEC split (§5.11) already bought the headroom once and cannot be repeated. |
| R14 | Continuity Camera photos are too poor for even the QR profile | low | Phone profile defaults to 500–600 µm modules; the phone path is compatibility, not the main route. |

---

## 17. Open questions, ranked by how much the answer changes the design

**OQ-1. What is the minimum reliable cell size on commodity laser printers and flatbed
scanners?** Changes: everything. It sets the density ratio (§13.4), which decides whether
the native raster ships at all. *Answered by:* Phase 0. *Until then:* build behind the
symbology interface so the pivot stays cheap.

**OQ-2. Is ink inset required, and does it force the vector render path?** Changes: PDF
size by roughly 20×, print time, and whether image-mask rendering survives. Interacts
with OQ-1 because inset only matters at small cell sizes. *Answered by:* Phase 0,
criteria 3 and 4.

**OQ-3. Should parity groups span the whole document, or should each page be
independently recoverable?** Changes: the user-facing promise and the reassembler.
Document-wide groups maximise efficiency; per-page groups would let a single surviving
sheet yield its own contents. The current choice is document-wide, which is right for
"one sheet was lost" and wrong for "I only found one sheet". *Answered by:* a product
decision in Phase 2, informed by which failure users actually describe.

**OQ-4. Does the header band need to be 25 mm?** Changes: capacity by 8–10% across every
table in §13. Driven by whether the descriptor QR must be phone-readable — if flatbed-only
is acceptable, 0.25 mm modules shrink it to 16 mm and the band could be 18 mm.
*Answered by:* Phase 1, once phone-readability of the descriptor is tested.

**OQ-5. Is `zstd` acceptable as the default given the twenty-year requirement, or should
`deflate` be the default with zstd as the opt-in?** Changes: sheet counts by 10–25%, and
the strength of the archival claim. Lowered in stakes by the reference decoder not
decompressing (§5.11). *Answered by:* a judgement call before Phase 1 ends.

**OQ-6. Should Deckle support appending parity to an existing archive later?** Changes:
the FEC choice — this is RaptorQ's decisive advantage (§9.3) and the only thing that
would reopen that decision. *Answered by:* product judgement; deferred to v2 unless it
turns out to be a headline feature.

**OQ-7. Multi-document sheets — should a small file share a sheet with another?**
Changes: layout engine and reassembler. Probably not worth it; noted so it is not
rediscovered.

**OQ-8. Should the QR profile support a "print-only-what-changed" incremental mode?**
Changes: nothing in v1, but it interacts with document identity and would want designing
before the format freezes. *Answered by:* explicitly deferred, with reserved bits in the
document header.

---

# 18. Colour mode — v1.1 "Chroma"

An optional mode that adds cyan, magenta and yellow ink planes to the native raster.
Specified here as a distinct version so that v1.0 can ship, and freeze its format, without
waiting on it. Colour mode is a **capacity mode, not an archival mode**, and §18.8 makes
that distinction binding rather than advisory.

## 18.0 The honest number, first

The intuitive claim is that four inks give four bits per cell and therefore 4× the
density. Two things reduce that, and both are structural rather than incidental:

**Ceiling is 3×, not 4×.** A colour scanner produces exactly three numbers per pixel
(R, G, B). Four ink layers are four unknowns recovered from three measurements — an
underdetermined system (§18.1). K is therefore excluded from the data alphabet, leaving
CMY: eight states, **3 bits per cell**.

**Realised gain is roughly 2×, not 3×.** Colour cells must be larger than mono cells
because of inter-plane registration error (§18.4), and colour needs one ECC level more
because an eight-state classifier errs more often than a two-state one (§18.2).

| Scenario | Mono baseline | Colour | Gain |
|---|---|---|---|
| Colour holds the same cell size, same ECC | 169 µm, Q, 136.3 KiB | 169 µm, Q, 404.7 KiB | **2.97×** |
| Colour holds cell size, one ECC level stronger | 169 µm, Q, 136.3 KiB | 169 µm, H, 334.3 KiB | **2.45×** |
| **Expected:** one cell step coarser, one ECC level stronger | 169 µm, Q, 136.3 KiB | 212 µm, H, 213.6 KiB | **1.57×** |
| Expected, if ECC can stay at Q | 169 µm, Q, 136.3 KiB | 212 µm, Q, 258.5 KiB | **1.90×** |
| Pessimistic: two cell steps coarser | 254 µm, Q, 60.5 KiB | 423 µm, H, 53.5 KiB | **0.88×** |

The pessimistic row is not decoration. If registration error forces colour cells two
steps coarser, **colour is worse than mono** — three bits in a cell of nine times the area
loses to one bit in a small cell. Colour mode lives or dies on §18.4, and Phase 0 gains a
measurement task because of it (§18.15).

Stated plainly for planning purposes: **expect colour to roughly double capacity, at the
cost of the twenty-year archival promise.** That is a real gain and worth building. It is
not a 4× gain, and no arrangement of the physics produces one.

## 18.1 Why CMY, and why K is excluded from the data alphabet

Three independent arguments, any one of which is sufficient.

**Measurement count.** An RGB scanner yields three values per pixel. Recovering four
independent ink coverages from three measurements requires the inks' spectra to be
linearly independent in a way three broad sensor bands can separate. Carbon black is
spectrally flat — it absorbs uniformly, so K's direction in RGB space is approximately
(−1, −1, −1). A C+M+Y overprint is also approximately neutral, so it occupies nearly the
same direction. The two are near-degenerate at 8 bits per channel with scanner noise.

**Alphabet collapse.** Even ignoring the measurement problem: of the 16 CMYK
combinations, the eight with K = 1 all read as near-black regardless of what lies beneath,
because K is opaque and laid last. Sixteen combinations therefore present as at most nine
distinguishable states, and CMY = 111 is itself near-black, collapsing to eight.
log₂(8) = 3 bits. The fourth ink buys nothing.

**Driver interference — the practical killer.** Consumer print pipelines apply grey
component replacement: the driver detects near-neutral CMY combinations and substitutes K
to save ink and reduce ink loading. A driver doing its job will silently rewrite our
C+M+Y cells as K cells and, under undercolour addition, do the reverse. **This is not a
tuning problem; it is the driver correctly destroying our encoding.** Defeating it
requires emitting device-native separations with colour management disabled — in PDF, a
`/DeviceN` colourspace with named colourants `[Cyan Magenta Yellow]` rather than
`/DeviceRGB` or a managed `/DeviceCMYK`. Whether commodity macOS drivers honour that is
an open question and a Phase 0 test (§18.15, R17).

**Decision.** *Options:* CMYK four-plane; CMY three-plane; CMY plus K as a fourth
low-confidence plane. *Choice:* **CMY three-plane, 3 bits per cell. K is reserved
exclusively for structure** — fiducials, sync dots, the descriptor QR, the human header,
the bootstrap page. *Rationale:* the three arguments above; and reserving K for structure
means page location, orientation and descriptor reading all work before any colour
calibration exists, using the same code as mono. *Reversibility:* high — `ink_planes` is a
bitmask field in the page descriptor, so a fourth plane is a format-version bump, not a
redesign, should someone find a way to separate it.

## 18.2 State alphabet and bit mapping

Eight states, one per vertex of the CMY cube. The bit mapping is the direct one:

| State | c m y | Appearance | Bits |
|---|---|---|---|
| 0 | 0 0 0 | paper white | 000 |
| 1 | 1 0 0 | cyan | 100 |
| 2 | 0 1 0 | magenta | 010 |
| 3 | 0 0 1 | yellow | 001 |
| 4 | 1 1 0 | blue | 110 |
| 5 | 1 0 1 | green | 101 |
| 6 | 0 1 1 | red | 011 |
| 7 | 1 1 1 | composite black | 111 |

The direct mapping is already optimal, and deliberately so: the dominant confusion is
"is ink *X* present or not", which is a single-ink error, and a single-ink error is
exactly a one-bit error under this mapping. No Gray recoding is needed or wanted — an
indirect mapping would turn one-ink confusions into multi-bit errors.

**The hardest discriminations are both one bit apart,** which is the property that makes
this work: white ↔ yellow (yellow absorbs only in blue, and blue is typically the
noisiest scanner channel), and composite black ↔ each of blue/green/red.

Yellow deserves a note. Office paper usually contains optical brightening agents that
fluoresce blue under the scanner's lamp, raising the white point's blue value and
*improving* white-versus-yellow separation — but by an amount that depends on the lamp's
UV content and on how much the paper's OBAs have degraded with age. It is a contrast
source we benefit from and must not depend on. The per-page calibration lattice (§18.5)
measures the actual separation rather than assuming it.

**Default symbol ECC for colour mode is H** (RS(255,159)), one level above mono's Q,
because per-cell classification error rates are higher for an eight-state decision than a
two-state one at equal optical SNR. A profile verified by round trip may drop to Q, and
§18.0 quantifies what that is worth.

## 18.3 Per-plane codeword assignment

This is the most consequential design decision in colour mode after registration, and it
has a clearly correct answer.

*Options:* (a) interleave all three planes into a common codeword pool; (b) give each
plane its own disjoint set of codewords.

*Choice:* **(b) — each ink plane carries its own disjoint, contiguous run of codewords.**
A page's `block_count` blocks are split into three equal runs assigned to C, M and Y in
that order; each run is interleaved within its own plane by the affine permutation of
§5.6, using plane-specific coefficients derived from the descriptor's `interleave_a`.

*Rationale:* consider the characteristic colour failure — yellow fades faster than cyan
and magenta, which is the normal behaviour of every consumer ink and toner set. Under
option (a), a lost yellow plane puts one bit in three *wrong in every codeword*: a 33%
symbol error rate against a 19% correction capacity at level H. Total, unrecoverable
loss. Under option (b), a lost yellow plane erases exactly one third of the blocks and
leaves the other two thirds pristine — which the cross-block FEC of §5.7 can rebuild
given `parity_ratio ≥ 0.5`. The difference between the two options is the difference
between a graceful degradation path and a cliff.

The partial-fade case matters more than the total-loss case, and option (b) wins there
too: a degraded yellow plane raises the erasure rate only within yellow's own codewords,
where the retry ladder's erasure flagging (§5.8) absorbs it locally instead of spending
every codeword's correction budget at once.

*Consequence for the user:* colour mode's parity guidance differs from mono's. The GUI
says so directly — *"At 50% parity, colour archives survive the complete loss of one ink
colour. Below that, they do not."* Default `parity_ratio` in colour mode is **0.5**, not
0.2, and that overhead is included in the estimator's sheet count.

*Reversibility:* high — the split rule is implied by `ink_planes` and `block_count` in the
descriptor; no new fields.

## 18.4 Registration — the constraint that decides colour mode

Ink planes do not land on top of each other. Colour laser engines expose each plane from
a separate drum and register mechanically; inkjets lay planes from different nozzle rows,
often on different passes and in alternating directions. Typical inter-plane registration
error is **±0.05 mm on a well-aligned inkjet and ±0.10 mm on a colour laser**, with cheap
units specified as loosely as ±0.15 mm.

Cell sampling reads the central 50% of each cell (§5.8), so a plane displaced by more
than 25% of the cell pitch pushes the wrong ink into the sampled region. Uncorrected,
that sets a floor of **cell ≥ 4 × registration error**: 0.2 mm for a good inkjet, 0.4 mm
for a typical colour laser. The 0.4 mm floor is where colour loses to mono outright
(§18.0, pessimistic row).

**Mitigation: per-plane affine correction.** Most registration error is systematic —
a constant offset plus small scale and rotation differences between planes — and
systematic error is measurable and removable. Each of the four corner fiducial positions
carries, alongside the K fiducial, three small three-armed registration marks printed one
per plane, at known offsets. Twelve marks in total yield a full affine fit per plane
(offset, scale, rotation, shear), applied before sampling.

What survives is the *local, random* component — nozzle-to-nozzle scatter, paper feed
jitter, drum eccentricity — which is expected to be ±0.02–0.04 mm, implying a floor
around 0.08–0.16 mm. **If that expectation holds, colour cells can sit at the same size as
mono cells and the gain is the full 2.45–2.97×.** If it does not, colour steps one size
coarser and the gain is 1.57–1.90×. If residual error exceeds 0.06 mm, colour mode is not
worth building for that hardware class.

That three-way branch is exactly what Phase 0 must resolve, and it is why colour is a
separate version rather than a v1.0 feature: the measurement that decides its shape
cannot be made before the mono ladder exists.

## 18.5 On-page colour calibration

An eight-state classifier needs to know where the eight states actually sit in scanner
RGB — which depends on the printer, the ink set, the paper, the scanner's lamp and
sensor, and the age of the page. None of these can be assumed, and the last one cannot
even be known at encode time.

**Calibration lattice.** Every 64 × 64 cells, a 4 × 4-cell patch is printed in a known
state, cycling through all eight. Cost: 16 / 4096 = **0.39%** of cells. On A4 at 169 µm
this places about 374 patches across the page, roughly 47 per state, giving the decoder a
spatially resolved measurement of every state's appearance. The classifier is fitted
locally and interpolated between patches, exactly as the sync-dot displacement field is
(§5.8) — the same machinery, a different quantity.

**Header strip.** A nine-patch strip (the eight states plus a K reference) in the header
band, 4 mm per patch, for the coarse global fit and for human inspection.

The decisive property: **the calibration patches age with the data.** A page whose yellow
has faded 30% over fifteen years has calibration patches whose yellow has faded 30% too,
so the classifier's decision boundaries move with the ink instead of being anchored to a
factory assumption. This is the single mechanism that makes a colour archive readable
after long storage at all, and it is why calibration is printed rather than stored.

Total colour-specific structural overhead: 0.39% for the lattice, plus the header strip
which sits in the already-reserved band. The structure budget rises from **2.5% to 3.5%**,
and every capacity figure in §18.12 uses 3.5%.

## 18.6 Decoder pipeline deltas

Relative to §5.8, colour changes six steps and adds three:

```
  1. locate page              UNCHANGED - fiducials are K, read from luminance
  2. resolve orientation      UNCHANGED - K markers
  3. global homography        UNCHANGED
  4. read descriptor QR       UNCHANGED - K; ink_planes field selects colour path
+ 4a. verify colour scan      reject greyscale scans (see below)
+ 4b. per-plane affine fit    12 corner registration marks -> 3 affine transforms
  5. local warp               UNCHANGED (K sync dots), then applied per plane
+ 5a. fit local classifier    calibration lattice -> local RGB->(c,m,y) unmixing
  6. adaptive threshold       REPLACED by per-plane density thresholding
  7. sample cells             per plane, each at its own corrected position
  8. per-cell confidence      per plane: |density - threshold| / local contrast
  9. de-interleave            per plane, plane-specific coefficients
 10. RS decode                UNCHANGED - each plane's codewords decoded independently
 11. CRC32C verify            UNCHANGED
 12. report                   EXTENDED with per-plane margin and per-plane error rate
```

**Unmixing.** *Options:* nearest-centroid classification among the eight measured patch
centroids with a Mahalanobis metric; a local 3 × 4 linear map from RGB to ink density
followed by independent per-plane thresholds. *Choice:* **the local linear map.**
*Rationale:* it produces three independent per-plane confidences with the same shape the
mono pipeline already consumes, so the retry ladder, erasure flagging and reporting need
no colour-specific code. Nearest-centroid gives a marginally better classification but
yields a posterior over states that must then be marginalised into per-bit confidences —
more code, in the reference decoder as much as the product, for a small gain.
*Reversibility:* high, decoder-side only, improvable for old archives after the fact.

**Greyscale-scan rejection.** A user scanning a colour archive in greyscale gets an image
where R ≈ G ≈ B and the three planes are irrecoverably summed. This is a likely and
completely silent failure, so it is detected explicitly: if the per-pixel channel spread
across the calibration patches is below a threshold, the decoder refuses the page and
says *"this page was scanned in greyscale; rescan in colour."* Colour management must
likewise be off or a known profile embedded; an unmanaged scan with an unknown profile
is accepted, because the calibration lattice absorbs an unknown but consistent transform.

**Per-plane margin reporting** is what makes fade visible before it becomes loss. A
recovery report reading *"cyan 22%, magenta 26%, yellow 71% of correction capacity
consumed"* tells the user their yellow is going and they have one reprint left. The GUI
surfaces exactly that, and it is the strongest argument for building the per-plane
reporting properly rather than averaging.

## 18.7 What stays black, always

Non-negotiable, because each of these must be readable before any colour processing is
possible, or must outlive the colour inks:

- Corner fiducials and the orientation marker.
- The sync-dot lattice.
- The page descriptor QR.
- The human-readable header, including the colour-mode warning.
- The header calibration strip's K reference patch.
- **The entire bootstrap page** (§12.5), always, without exception.

A colour archive whose colour has failed completely still presents a page that locates,
orients, and announces what it was — and a bootstrap page that explains how to read it.
That is the difference between a degraded archive and an unidentifiable sheet of confetti.

## 18.8 Archival policy

Colour mode weakens the central promise of this project, and the plan should say so
rather than bury it in a caveat.

Carbon-black toner is effectively permanent: it is a fused thermoplastic carbon layer,
chemically inert, with no known fading mechanism on a hundred-year scale. Colour is not
in that category. Dye-based inkjet inks fade visibly in years under ambient light and can
shift measurably in dark storage. Pigment inkjet inks are far better but fade
*differentially*, and yellow pigments are generally the least lightfast. Colour toner is
stable but its yellow is the weakest component and its fusing is less complete than K's.

Therefore, as policy rather than guidance:

1. **Colour mode is never the default,** at any density tier, on any medium.
2. **Every colour page prints `COLOUR MODE — NOT RATED FOR LONG-TERM ARCHIVE`** in the
   human header, in K, at the same size as the page number.
3. **Dye inks are refused, not warned about.** `medium.ink_class = "dye"` combined with
   `density.ink_planes = "cmy"` is a configuration error with an explanatory message, not
   a checkbox. Pigment and toner are permitted; pigment carries a warning.
4. **Default `parity_ratio` is 0.5** in colour mode (§18.3), so that the loss of one ink
   is survivable rather than fatal.
5. **The estimator reports an archival class**, not just a sheet count: `archival` for
   K-only on laser toner and acid-free paper, `durable` for K-only otherwise,
   `capacity` for any colour configuration. The GUI shows it beside the sheet count with
   the same prominence as the provenance mark.
6. **A recommended re-verification interval** is printed on colour pages: rescan within
   five years. Because per-plane margin is reported (§18.6), re-verification is
   informative rather than ritual — it tells the user whether their yellow is failing
   while there is still time to reprint.

The honest summary, which belongs in the user documentation verbatim: *colour roughly
doubles what fits on a sheet, and roughly halves the number of years you should trust
that sheet.*

## 18.9 Format deltas

Colour is a **format version bump to 0x0110**, backward-compatible in the direction that
matters: a v1.1 decoder reads v1.0 archives unchanged, and a v1.0 decoder encountering
`ink_planes ≠ 0` fails with `DKL_E_UNSUPPORTED` and a message naming the required
version, rather than misreading the page.

Page descriptor additions (§12.2), placed in the reserved region:

```
 off  size  field
  78     1  ink_planes       bitmask: bit0=C bit1=M bit2=Y; 0 = K-only (v1.0 behaviour)
  79     1  cal_period       u8, cells between calibration patches (64)
  80     1  cal_patch_cells  u8, patch edge in cells (4)
  81     1  plane_reg_spec   u8, 1 = twelve three-armed corner marks
  82     1  colour_ecc       u8, RS k for the colour planes if it differs from rs_k
  83     3  reserved
  86   ...  age_header       unchanged, still last
```

These occupy the eight bytes v1.0 reserves at offset 78 (§12.2), which is why v1.0
reserved them: a v1.0 encoder writes zeros there, and `ink_planes == 0` is exactly the
K-only behaviour a v1.0 decoder already implements.

`block_count` retains its meaning; §18.3's rule — three equal contiguous runs assigned to
C, M, Y in order — is implied by `ink_planes` having three bits set and needs no field.

Document header (§12.1) is unchanged. Colour is a page-level property, so an archive may
in principle mix colour data pages with K-only pages; the bootstrap page always is one.

## 18.10 Symbology registry

§6 forbids per-symbology branches outside a plugin, and colour must not smuggle branches
into the layout engine or estimator through a back door.

*Options:* (a) a `ColorMode` parameter threaded through `Symbology::plan_region`;
(b) a separate symbology plugin with its own id; (c) one implementation registered twice
under two ids.

*Choice:* **(c).** `raster_k` (id 1) and `raster_cmy` (id 3) are two registry entries over
one implementation, differing in their constructor parameters.

*Rationale:* the two genuinely differ in everything the registry exists to declare —
`reader_requirements` returns a colour-capable flatbed for one and any flatbed for the
other; `density_range` differs; default ECC differs (H versus Q); default parity ratio
differs (0.5 versus 0.2); archival class differs. Threading a parameter (option a) would
push those differences into every caller as conditionals, which is precisely the
special-casing §6 exists to prevent. A wholly separate plugin (option b) would duplicate
the 95% of code the two share. Two registrations over one implementation puts the
differences in the registry, where the rest of the system already looks for them.

`RegionPlan` gains `bits_per_unit: u8` (1 or 3), which the chunker uses and nothing else
interprets. `SymbolDecode` gains `per_plane: Option<[PlaneReport; 3]>`, which the recovery
report renders and nothing else interprets.

*Reversibility:* high.

## 18.11 Configuration deltas

```toml
[density]
ink_planes      = "k"          # k | cmy    -- default k, never auto-selected

[medium]
ink_class       = "toner"      # toner | pigment | dye
                               # dye + cmy is refused (18.8)

[reader]
color           = false        # required true for cmy; validated at plan time

[ecc]
symbol_level    = "Q"          # colour mode defaults to "H"
parity_ratio    = 0.20         # colour mode defaults to 0.50 (18.3)

[render]
separations     = "devicen"    # devicen | cmyk_unmanaged
                               # cmy only; how separations reach the driver (18.1)
```

Colour density tiers, all placeholders until Phase 0 (§18.15) replaces them:

| Tier | toner (colour laser) | pigment inkjet |
|---|---|---|
| conservative | 339 µm | 254 µm |
| balanced | 254 µm | 212 µm |
| aggressive | 212 µm | 169 µm |

The colour ladder starts one to two steps coarser than the mono ladder at every tier,
which is the registration allowance of §18.4 expressed as a default. Where Phase 0 shows
per-plane affine correction removes more error than expected, these move down.

## 18.12 Capacity — colour

> Same standing caveat as §13, and one more: these assume the eight states are reliably
> separable at the stated cell size, which no measurement yet supports.

Structure budget 3.5% (§18.5). 3 bits per cell. **Level H is the colour default**;
the Q column applies only to round-trip-verified profiles.

**A4, CMY**

| Cell | Grid | Cells | Raw | Q | **H (default)** | vs mono Q, same cell |
|---|---|---|---|---|---|---|
| 169 µm | 1090 × 1456 | 1,587,040 | 581.2 KiB | 404.7 KiB | **334.3 KiB** | 2.45× |
| 212 µm | 871 × 1164 | 1,013,844 | 371.3 KiB | 258.5 KiB | **213.6 KiB** | 2.45× |
| 254 µm | 726 × 970 | 704,220 | 257.9 KiB | 179.6 KiB | **148.3 KiB** | 2.45× |
| 339 µm | 544 × 727 | 395,488 | 144.8 KiB | 100.8 KiB | **83.3 KiB** | 2.45× |
| 423 µm | 436 × 582 | 253,752 | 92.9 KiB | 64.7 KiB | **53.5 KiB** | 2.45× |

**Letter, CMY**

| Cell | Grid | Raw | Q | **H (default)** |
|---|---|---|---|---|
| 169 µm | 1125 × 1352 | 557.0 KiB | 387.9 KiB | **320.4 KiB** |
| 212 µm | 899 × 1081 | 355.9 KiB | 247.8 KiB | **204.7 KiB** |
| 254 µm | 749 × 901 | 247.1 KiB | 172.1 KiB | **142.2 KiB** |
| 339 µm | 561 × 675 | 138.7 KiB | 96.6 KiB | **79.8 KiB** |
| 423 µm | 450 × 540 | 89.0 KiB | 62.0 KiB | **51.2 KiB** |

**A3, CMY**

| Cell | Grid | Raw | Q | **H (default)** |
|---|---|---|---|---|
| 169 µm | 1604 × 2183 | 1282.3 KiB | 892.9 KiB | **737.6 KiB** |
| 212 µm | 1282 × 1745 | 819.2 KiB | 570.5 KiB | **471.2 KiB** |
| 254 µm | 1069 × 1455 | 569.6 KiB | 396.6 KiB | **327.6 KiB** |
| 339 µm | 801 × 1090 | 319.7 KiB | 222.6 KiB | **183.9 KiB** |

**Read these tables against §18.0, not on their own.** The last column of the A4 table is
a same-cell-size comparison and therefore an upper bound; the operative comparison is
mono at its best working cell size against colour at *its* best working cell size, which
is the third row of §18.0's table and reads **1.57×**.

And one number that must not be lost: at 50% parity (§18.3) versus mono's 20%, colour's
*net* advantage after parity sheets is 1.57 × (1.20 / 1.50) = **1.26×** for an archive
configured to survive an ink failure. Colour's headline gain and its delivered gain are
different numbers, and the estimator shows the delivered one.

## 18.13 Reference decoder impact

Colour adds roughly 60–90 lines to the decode path: RGB load and channel handling, the
per-plane affine fit from the twelve corner marks, the local unmixing fit, and per-plane
thresholding. Added to the ~380-line mono decoder, that overruns the ~400-line budget.

**Decision.** *Options:* extend `dkl_ref.py` and raise the budget; ship a third module.
*Choice:* **`dkl_ref.py` remains mono-only and unchanged; colour archives print a third
module, `dkl_ref_cmy.py` (~200 lines, NumPy and Pillow only), on the bootstrap page with
its own printed SHA-256.** *Rationale:* the mono reference decoder is the artefact the
archival promise rests on, and it must not grow to carry a mode that is explicitly not
archival. A recoverer holding a K-only archive should never have to read past the first
module. *Reversibility:* high — it is a separate file and a separate QR block.

The bootstrap page of a colour archive therefore carries three modules. It will spill to
two pages, which §12.5 already permits and which costs nothing worth protecting.

## 18.14 Calibration ladder additions

The ladder gains three colour tests, all on one additional sheet:

- **Registration vernier.** A per-plane vernier pattern, readable both by eye and by
  machine, measuring inter-plane registration error in micrometres, before and after
  per-plane affine correction. This is the measurement that sets the colour cell size.
- **State separability.** All eight states at each cell size in the ladder, with
  classification error rate reported per state. Yellow-versus-white and composite-black-
  versus-blue/green/red are reported separately, since they set the floor.
- **Separation integrity.** A patch that is C+M+Y overprint adjacent to a patch that is
  K, printed through the configured `render.separations` path. If the scanner cannot
  distinguish them, the driver has applied grey component replacement and colour mode is
  unusable on that printer (§18.1, R17). This test runs first and short-circuits the rest.

## 18.15 Milestones

**Phase 0 additions** (colour ladder, +1 week, run alongside the mono ladder):

- Measure inter-plane registration error on both printers, raw and after per-plane affine
  correction. **This sets the colour cell size and therefore whether colour is worth
  building at all.**
- Verify that a `/DeviceN` separations path survives commodity macOS drivers without grey
  component replacement.
- Measure eight-state classification error rates at each cell size on both paper types.

**Phase 0 colour exit criteria** — all three required before Phase 6 is scheduled:

1. Residual per-plane registration error after affine correction is **≤ 0.04 mm**.
2. Separations survive the driver: C+M+Y and K are distinguishable in the scan.
3. The measured gain over mono at each hardware's best working cell size is **≥ 1.5×**,
   comparing like ECC and like parity.

Criterion 3 is deliberately a lower bar than mono's 3× (§14), because colour is an
increment on a shipped product rather than a premise the project rests on. If it fails,
colour mode is dropped and nothing else changes — which is the reason it is v1.1.

**Phase 6 — colour mode (5–6 weeks), after v1.0 ships.**

Format v0x0110. Registry entry `raster_cmy`. Per-plane registration marks, calibration
lattice, unmixing, per-plane codeword assignment and reporting. `/DeviceN` render path.
Colour ladder analyzer. `dkl_ref_cmy.py`. Colour acquisition, including greyscale-scan
rejection. GUI: colour mode with its archival-class warning and per-plane margin view.

*Exit:* a colour archive round-trips through the software loop; a colour archive with the
yellow plane fully removed recovers completely at 50% parity; and a hardware-loop print
and scan on both Phase 0 printers meets the measured gain from criterion 3.

## 18.16 Test matrix additions

Added to §15.1, crossed with `{k, cmy}` and, for colour, `{169, 212, 254, 339 µm}`:

| Degradation | Parameters swept |
|---|---|
| Per-plane registration offset | 0, 0.02, 0.04, 0.08, 0.15 mm, independent per plane, plus rotation ±0.2° |
| Single-plane fade | 10%, 30%, 60%, 100% density loss, each plane in turn |
| Differential fade | yellow at 2× and 4× the rate of cyan and magenta |
| Colour cast | white point shifted ±10% per channel (scanner lamp ageing) |
| Channel noise | blue channel at 2× and 4× the noise of red and green |
| Greyscale scan | full channel collapse — must be *detected and refused*, not decoded wrongly |
| Grey component replacement | C+M+Y cells rewritten as K — must be detected and reported |
| Ink bleed across cells | 5%, 10%, 20% of cell width, per plane |
| Scanner colour profile | unmanaged, sRGB, AdobeRGB, and a deliberately wrong profile |

Added to §15.2 invariants:

- **Plane loss recovers.** With `parity_ratio ≥ 0.5`, removing any one plane entirely
  from every page recovers the archive completely — asserted, not sampled.
- **Plane interleave is disjoint.** No codeword's bits span two planes, asserted
  structurally at encode time. This is what §18.3 depends on and it must not decay.
- **Greyscale refusal.** A greyscale render of a colour page produces a clear refusal,
  never a partial or wrong decode.
- **Mono decoder rejects colour cleanly.** A v1.0 decoder given a v1.1 colour page returns
  `DKL_E_UNSUPPORTED` naming the version, never a misread.
- **K structure is colour-free.** Fiducials, sync dots, descriptor QR, header and
  bootstrap page contain no C, M or Y ink — asserted on the rendered separations.

## 18.17 Risk register additions

| # | Risk | Severity | Mitigation |
|---|---|---|---|
| R15 | **Inter-plane registration error forces colour cells so coarse that colour loses to mono** | critical | §18.4; per-plane affine correction; Phase 0 criterion 1 with a ≤ 0.04 mm threshold; colour is droppable at zero cost to v1.0. |
| R16 | **Differential fading destroys colour archives within the promised lifetime** | critical | Colour is declared non-archival as policy (§18.8); dye inks refused; per-plane codewords give graceful degradation (§18.3); per-plane margin reporting warns before loss; 5-year re-verification printed on the page. |
| R17 | **Printer driver grey component replacement silently rewrites C+M+Y as K** | high | `/DeviceN` separations path; separation-integrity test runs first in the ladder and short-circuits (§18.14); Phase 0 criterion 2. |
| R18 | User scans a colour archive in greyscale and gets silent garbage | high | Explicit detection and refusal (§18.6); asserted as an invariant (§18.16). |
| R19 | Scanner colour management alters values unpredictably between scans | medium | Calibration lattice absorbs any consistent transform (§18.5); acquisition disables management where the driver allows and records what it could not control. |
| R20 | Yellow-on-white contrast depends on paper optical brighteners that degrade with age | medium | Contrast is measured per page from the calibration lattice rather than assumed; the lattice ages with the paper. |
| R21 | Ink bleed on plain paper smears colour boundaries far worse than mono boundaries | medium | Colour tiers default one to two steps coarser (§18.11); Phase 0 measures on both paper types; coated stock recommended, with the specular-reflection caveat. |
| R22 | Colour mode's complexity leaks into the v1.0 codebase before v1.0 ships | medium | Colour is v1.1 and Phase 6, after freeze; the only v1.0 accommodation is reserved descriptor bytes (§18.9). |

## 18.18 Open questions

**OQ-9. What is residual per-plane registration error after affine correction, on
commodity colour hardware?** Changes: whether colour mode is worth building, and if so
whether the gain is 1.57× or 2.45×. Ranks immediately after OQ-1 in consequence.
*Answered by:* Phase 0 colour ladder.

**OQ-10. Can grey component replacement be defeated on commodity macOS drivers?**
Changes: binary — colour mode is impossible on printers where it cannot. *Answered by:*
Phase 0 criterion 2. *Contingency:* if `/DeviceN` fails widely, investigate driving the
printer through a raw PPD/PostScript path, and accept a much narrower hardware list.

**OQ-11. Is a 1.26× net gain after colour's 50% parity worth the mode's complexity?**
This is a product question, not a technical one, and §18.12 poses it deliberately. The
counter-argument is that not every user wants ink-loss survivability: at 20% parity
colour delivers its full ~2×, and the user who is storing a large file for five years in
a drawer may rationally choose it. *Answered by:* a product decision before Phase 6 is
scheduled, informed by Phase 0's measured gain.

**OQ-12. Should colour mode support a mixed archive — colour data pages with a K-only
subset carrying the most critical files?** The format already permits it (§18.9). It
would let a user put a key or an index on archival-grade K pages and bulk data on colour
pages. Changes: layout engine and the manifest. *Answered by:* deferred; noted so it is
not rediscovered as a surprise.
