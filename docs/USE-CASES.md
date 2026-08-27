# What Deckle is for

Deckle answers one question in two different situations: **how do you keep data when the
machine holding it is not available to you?**

- **[Long-term archival](USE-CASE-ARCHIVAL.md)** — the machines are gone, unpowered, or
  can no longer read the medium. Fire, flood, EMP, or twenty years passing. The
  distinguishing feature is that every archive prints its own decoder, so the paper
  outlives the software.
- **[Carrying data through a device inspection](USE-CASE-TRANSPORT.md)** — the machine
  can be examined, imaged or kept, and the material still has to arrive. Paper travels
  differently from devices; it is opaque, not hidden.

The two want opposite settings in places, which is why they are separate documents. An
archive wants black toner and enough parity to lose a whole sheet; a courier run wants
colour for the capacity, because inks fading over decades does not matter to something
that lives for a week.

What follows is the reference both share.

## What belongs on paper

Think *irreplaceable and small*, not *the whole file server*. Deckle stores tens to low
hundreds of kilobytes per sheet. Photographs, video and disk images belong on offline
drives. Paper is for the root of trust and the configuration that everything else depends
on.

**Keys and identity.** An encrypted password vault. GPG, SSH, `age` and TLS private keys
plus revocation material. A small organisation's internal CA. Two-factor backup codes.
Wallet seeds together with the descriptors and address list you need to actually use them
— a seed with no descriptor is often not enough.

**The configuration that restarts things.** Generator, well pump, inverter and
water-treatment setpoints. PLC and RTU config exports. Radio plans: frequencies, tones,
repeater lists, net scripts. The one-page procedure for bringing the well back up. These
are usually a few kilobytes of text that took someone a decade to get right.

**Documents you would have to litigate to replace.** Deeds, insurance policies, wills,
powers of attorney, licence numbers.

**Enough context to know what you recovered.** File names, hashes and a short plain-text
README travel inside the archive; Deckle verifies a SHA-256 over the whole set on decode
and tells you whether it matched.

## Capacity

Net bytes per sheet — after Reed–Solomon error correction, block framing, the corner
markers, the sync lattice and (in colour) the calibration patches. Nothing further is
deducted. Cross-block parity adds *sheets* rather than reducing these figures.

| Cell size | A4 black | A4 `cm` | A4 `cmy` | Letter black | Letter `cm` | Letter `cmy` |
|---|---|---|---|---|---|---|
| 254 µm | 61 kB | 121 kB | 182 kB | 59 kB | 117 kB | 176 kB |
| 212 µm | 88 kB | 176 kB | 264 kB | 85 kB | 169 kB | 254 kB |
| 169 µm | 139 kB | 277 kB | 416 kB | 133 kB | 265 kB | 397 kB |
| 127 µm | 246 kB | 492 kB | 737 kB | 236 kB | 472 kB | 708 kB |

Compression runs first, so text and configuration usually need fewer sheets than this
implies. Encrypted payloads are incompressible, so for those the table is exact. Run
`deckle estimate` on the real files: it is the layout engine in dry-run mode, not a
formula that can drift from it.

## The settings

**Ink.**

| | bits/cell | capacity | durability |
|---|---|---|---|
| `--ink k` (default) | 1 | 1× | **archival.** Fused carbon black, no known fading mechanism on a hundred-year scale |
| `--ink cm` | 2 | 2× | cyan and magenta only |
| `--ink cmy` | 3 | 3× | most capacity, least durable |

`--ink cm` leaves out yellow, which is the weak link twice over: the least lightfast ink
in almost every set, and read in the blue channel, the noisiest a scanner has. Measured,
it is untouched by any amount of blue-channel noise or yellow fade, and tolerates half
again as much ink crosstalk as `cmy`. If you are using colour at all, it is usually the
better trade.

Neither colour mode is rated for long-term storage, both need a colour scan to read back,
and the reference decoder printed on the bootstrap page refuses colour pages by design.

**Cell size.** Start at `--cell 254`. It works on mediocre hardware and it is the default.
Move to 169 µm only after a real round trip on your own printer and scanner, checking that
the reported correction margin says *healthy*. `deckle simulate` models damage but cannot
tell you what your printer's dot gain is.

**Parity.** `--parity 0.2` protects against damage within sheets. To survive losing a
*whole* sheet you need roughly `--parity 1/(N-1)` where N is the total sheet count: half
for a three-sheet archive, a fifth for a six-sheet one. `deckle estimate` prints the real
answer — *"any 1 of 3 sheets may be destroyed or missing"* — and that line is the one to
read.

**Documentation.** Leave the bootstrap page on unless you control both ends and will have
Deckle at the other one. Without it the sheets can only be read by Deckle itself, which is
the dependency the project exists to avoid.

## Storing paper

- **Laser toner on acid-free paper.** Not dye inkjet: dye fades in years under light.
- **Two buildings, or one building and one trusted person.** Copies defeat more failure
  modes than any parameter does.
- **A fire safe or a dry box.** Paper's real enemies are fire, water, UV and insects, and
  error correction addresses none of them.
- **Verify once, now.** Decode your own scans and check the hash before you file it away.
  An archive nobody has ever read is a wish, not a backup.

## Limits that apply to both

**Deckle does not encrypt anything yet.** [PLAN.md](PLAN.md) §9.6 specifies `age` and the
format reserves a field, but the tool does not implement it. **Encrypt before you encode.**
A paper backup of a plaintext seed phrase or password database is a bearer instrument:
whoever handles it has it.

**Phone cameras cannot read the dense grid.** A flatbed scanner reads data sheets; a phone
reads the bootstrap page. Confirm a scanner will be available before you rely on this.

**No hardware measurement has been made.** Every capacity figure here comes from a
software loop. The printer-and-scanner testing that would confirm the density tiers —
Phase 0 in [PLAN.md](PLAN.md) §14 — has not been done. See
[PROTOTYPE.md](PROTOTYPE.md) for what is actually built and what it measurably survives.

**This is not a disk replacement.** Anything measured in gigabytes belongs on offline
drives; paper holds the keys and the configuration that let you use them.
