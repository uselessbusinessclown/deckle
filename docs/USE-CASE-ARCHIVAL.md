# Use case: long-term archival that outlives the machines

*The other use case is [carrying data through a device search](USE-CASE-TRANSPORT.md).
Shared settings and storage advice are in [USE-CASES.md](USE-CASES.md).*

## The failure this addresses

Not a failed disk — that is what backups are for. This is for the case where **the
machines are gone, or unpowered, or simply cannot read the medium any more**:

- A house fire that takes the NAS, the laptop and the drive in the desk drawer.
- Flood, or a lightning strike through the mains.
- A severe geomagnetic storm or a high-altitude EMP: whatever was powered and connected
  is damaged; whatever was stored unpowered in a metal box probably is not.
- Twenty years passing. The drive still exists but nothing has the interface, the
  filesystem driver, or the proprietary format's software.

Paper is not clever, but it is **passive**. There are no semiconductors to damage, no
charge to leak away, no controller firmware to become unobtainable, no spinning parts, no
encryption chip that dies with its keys. Ink on paper is a chemical fact.

## Why not microfiche

Microfiche is denser, older and thoroughly proven, and if you are an institution filming
fifty thousand pages it remains the right answer. It is the wrong answer for a person,
because the *creation* and the *reading* both need specialised equipment. A reader-printer
is not something you can buy at any price point, keep switched off in a cupboard, and
replace in a hurry.

Deckle's bet is the opposite: every piece of the recovery stack should be a commodity.

| Piece | Role | Replaceable? |
|---|---|---|
| Printed sheets | the archive; passive, no power | you print more |
| Ordinary laser printer | to create it, once | any office |
| Flatbed scanner | reads the dense cell grid | £50, any electronics shop |
| Phone with a QR app | reads the bootstrap page | any phone |
| Python 3 | reconstructs the files | already on most systems |

Nothing else. No reader-printer, no darkroom, no service bureau, no proprietary software,
no internet, and no account with anyone.

The trade is capacity: tens to low hundreds of kilobytes per sheet. That is not a records
centre. It is enough for the material that everything else depends on.

## Future-proof decodeability

This is the part that distinguishes Deckle from "print your data as a barcode", and it is
worth being precise about, because the usual failure of paper-backup schemes is that the
paper outlives the software that reads it.

**Every archive prints its own decoder.** The last sheets of every archive are a
*bootstrap page* carrying, in this order:

1. **What this is**, in plain language — that the sheets are a backup, and that you do
   not need the program that made them.
2. **A numbered procedure**, assuming no internet and no prior knowledge: how to scan,
   at what settings, what to run.
3. **The page parameters in words** — grid dimensions, cell size, the Reed–Solomon
   parameters, the interleave band height, the compression used. Enough to reimplement
   the decoder from scratch if the printed one is somehow unusable.
4. **A nine-line program**, printed as text, that turns QR text back into a file. Short
   enough to retype by hand.
5. **The complete source of `dkl_ref.py`** as ordinary QR codes, with its SHA-256 printed
   beside them so you can check what you recovered.
6. **`dkl_fec.py`** likewise, needed only if sheets are missing or damaged.

The reference programs use the **Python standard library only**. No NumPy, no Pillow, no
`pip install` — `dkl_ref.py` decodes PNG itself, in about ninety lines using `zlib`. That
choice was made after finding that the stock Python on a current macOS has neither NumPy
nor Pillow, which is direct evidence that "just install the dependency" is a step a
recoverer may not manage. They are MIT licensed, and say so in their own headers, so
whoever finds them decades from now can see they are free to use.

**This has been demonstrated, not asserted.** The QR codes on a bootstrap page were read
with Apple Vision — a decoder that knows nothing about this project. The recovered
programs matched the SHA-256 printed beside them, were byte-identical to the files in the
repository, and then rebuilt a three-sheet archive that had one sheet destroyed. CI
repeats an equivalent check on every commit with an independent QR decoder, and separately
runs the Python decoder against real rendered pages.

**Two honest gaps.** `dkl_ref.py` was written alongside the encoder, so it proves the
format is *decodable* — it does not prove the format is *documented*. `docs/FORMAT.md`
and an outside reimplementation are still to come. And the reference decoder reads black
archives only; it refuses colour pages by name rather than misreading them, which is the
deliberate pairing with colour not being rated for long-term storage.

## Settings for archival

**Ink: black.** `--ink k`, the default. Fused carbon toner has no known fading mechanism
on a hundred-year scale. The colour modes triple or double capacity and are explicitly
not rated for long-term storage — and a colour archive is also the one kind the printed
reference decoder will not read. For an archive, take the capacity hit.

**Medium: laser toner on acid-free paper.** Not dye inkjet, which fades in years under
light. Toner is a fused thermoplastic carbon layer; it is the most stable printed material
there is.

**Cell size: `--cell 254`.** The conservative tier and the default. Only move to 169 µm
after a real round trip on your own printer and scanner, checking that the reported
correction margin says *healthy*. `deckle simulate` models damage but cannot tell you what
your printer's dot gain is.

**Parity: enough to lose a sheet.** `--parity 0.2` protects against damage within sheets.
To survive losing a *whole* sheet you need roughly `--parity 1/(N-1)` where N is the total
sheet count — half for a three-sheet archive, a fifth for a six-sheet one. `deckle
estimate` prints the real answer:

```
Loss tolerance     any 1 of 3 sheets may be destroyed or missing
```

That line covers fire, water, a torn page and an envelope going missing, not just exotic
failures. Read it before you print.

**Documentation: on.** Never `--no-bootstrap` for an archive. That flag exists for sheets
joining a binder that already has one.

## Sizing

Worst-case sheet counts, at 30% parity, measured on incompressible input. Real
configuration and text compress first and need fewer sheets; **encrypted payloads do
not compress at all**, so for anything you encrypt these figures are exact.

| Content | 254 µm | 169 µm |
|---|---|---|
| Keys, seeds, a few config files (up to 32 KiB) | 1 sheet | 1 sheet |
| Password database plus key material (256 KiB) | 6 sheets | 3 sheets |
| A document set (1 MiB) | 23 sheets | 10 sheets |
| 5 MiB | 113 sheets | 50 sheets |

Add three A4 sheets for the bootstrap page, once per archive.

Above about 200 sheets `deckle estimate` warns you, and it is right to: that is a signal
the material belongs on offline drives with paper holding only the keys to them.

## A concrete pack

For one person or a family, six to twelve A4 sheets covers the irreplaceable set:

- an **encrypted** password database
- key material: GPG, SSH, `age`, TLS private keys and revocation certificates
- wallet seeds **with** their descriptors and address list — a seed alone is often not
  enough to spend from
- two-factor backup codes
- a few pages of procedures: generator, well pump, inverter, water treatment setpoints;
  radio frequencies and repeater lists; the one-page "how to bring it back up"
- identity and property documents: deeds, insurance, wills, powers of attorney
- the bootstrap page

That fits in a legal envelope. Deckle verifies a SHA-256 over the whole set on decode and
tells you whether it matched, so you know the recovered blob is the right one.

## Storing it

- **Two buildings, or one building and one trusted person.** Copies defeat more failure
  modes than any parameter does.
- **A fire safe or a dry box.** Paper's real enemies are fire, water, UV and insects, and
  error correction addresses none of them.
- **If you are also storing the reading hardware** against the electronics being gone:
  a cheap flatbed scanner and a phone or single-board computer, stored unpowered in a
  metal container, the phone charged once a year. Put a one-page procedure on the outside
  of the envelope so it can be read by torchlight without decoding anything: *scan at 600
  dpi greyscale, sharpening off, then run `python3 dkl_ref.py scan-*.png -o recovered`.*
- **Verify once, now.** Decode your own scans and check the hash before you file it away.
  An archive nobody has ever read is a wish, not a backup.

## Limits, stated plainly

**Deckle does not encrypt anything yet.** [PLAN.md](PLAN.md) §9.6 specifies `age` and the
format reserves a field, but the tool does not implement it. **Encrypt before you print.**
A paper backup of a plaintext seed phrase or password database is a bearer instrument:
whoever opens the envelope has it. Use `age`, `gpg` or your password manager's own
encrypted export, and make sure the passphrase is recoverable by some other route — a
printed key you cannot decrypt is the same as no key at all.

**No hardware measurement has been made.** Every capacity figure in this project comes
from a software loop. The printer-and-scanner testing that would confirm the density
tiers — Phase 0 in [PLAN.md](PLAN.md) §14 — has not been done. Treat 254 µm as the safe
default until you have done your own round trip.

**Paper still burns.** Error correction handles a torn corner, a coffee ring, a crease and
a missing sheet. It does not handle the safe being in the fire. Copies, in separate
places, do.

**This is not a disk replacement.** Photographs, video and disk images belong on offline
drives in shielded storage. Paper is the root of trust and the small configuration that
lets you use everything else.
