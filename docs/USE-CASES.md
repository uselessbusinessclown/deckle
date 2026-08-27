# What Deckle is for

Deckle is a small answer to a narrow question: **which of your files still exist after
the electronics are gone?**

Not "gone" as in a failed disk — that is what backups are for. Gone as in the machines
are dead or unpowered and stay that way for a while: a house fire that takes the NAS and
the laptop, a flood, a lightning strike, a severe geomagnetic storm, a high-altitude EMP,
or simply twenty years passing and nothing being able to read the medium any more.

Paper is not clever, but it is passive. There are no semiconductors in a sheet of toner
on paper to damage, nothing to lose charge, no controller firmware to become
unobtainable, and no format to go out of support. What it costs you is capacity, and the
whole design is about making that trade honestly.

## The recovery stack

The reason to build this rather than buy microfiche is the kit you need on the other end.
Microfiche is denser and older and proven, but creating and reading it needs specialised
equipment. Deckle's bet is that the whole recovery stack should be things you can buy at
any price point, switch off, and put in a drawer:

| Piece | Role |
|---|---|
| The printed sheets | the archive itself; passive, no power |
| An ordinary laser printer | to create it, once |
| A flatbed scanner | reads the dense cell grid |
| A phone with any QR app | reads the bootstrap page |
| Python 3 | reconstructs the files |

Nothing else. No reader-printer, no darkroom, no service bureau, no proprietary software,
and no internet. The Python programs are printed on the archive itself as QR codes, use
the standard library only, and carry an MIT licence so whoever finds them may use them.

**The division of labour matters.** A phone camera reads QR — the bootstrap page, and
small payloads if you choose the QR symbology. A phone camera does **not** reliably read
the dense cell grid at 169 or 254 µm; that is a flatbed's job. Do not plan a recovery
around photographing data sheets with a phone.

## What belongs on paper

Think *irreplaceable and small*, not *the whole file server*. Deckle stores tens to low
hundreds of kilobytes per sheet. Photos, video and disk images belong on offline drives.
Paper is for the root of trust and the configuration that everything else depends on.

**Keys and identity.** An encrypted password vault. GPG, SSH, `age` and TLS private keys
plus revocation material. A small organisation's internal CA. Two-factor backup codes.
Wallet seeds together with the descriptors and address list you need to actually use
them — a seed with no descriptor is often not enough.

**The configuration that restarts things.** Generator, well pump, inverter and
water-treatment setpoints. PLC and RTU config exports. Radio plans: frequencies, tones,
repeater lists, net scripts. The one-page procedure for bringing the well back up. These
are usually a few kilobytes of text that took someone a decade to get right.

**Documents you would have to litigate to replace.** Deeds, insurance policies, wills,
powers of attorney, licence numbers.

**Enough context to know what you recovered.** File names, hashes and a short plain-text
README travel inside the archive; Deckle verifies a SHA-256 over the whole set on decode
and tells you whether it matched.

## Sizing it

Worst-case sheet counts, measured on incompressible input — real files with text or
configuration in them compress first and need fewer sheets. A4, black only.

| Content | 254 µm, 20% parity | 254 µm, 50% parity | 169 µm, 20% parity | 169 µm, 50% parity |
|---|---|---|---|---|
| 64 KiB | 2 sheets | 2 | 1 | 1 |
| 256 KiB | 6 | 7 | 3 | 3 |
| 1 MiB | 21 | 26 | 10 | 12 |

Add three A4 sheets for the bootstrap page, once per archive.

A useful first archive for a household or a small organisation is six to twelve sheets:
an encrypted password database, key material, a few pages of procedures, the identity
documents, and the bootstrap page. That fits in an envelope.

Run `deckle estimate` on your actual files before printing anything; it measures the real
compressed size and prints the real sheet count, because the estimator *is* the layout
engine rather than a formula that can drift from it.

## Choosing the settings

**Cell size.** Start at `--cell 254`. It is the conservative tier and it works on
mediocre printers. Only move to `--cell 169` after you have done a real round trip on
your own printer and scanner — print, scan, decode, and check the reported correction
margin says *healthy*. `deckle simulate` will tell you how a configuration behaves under
modelled damage, but it cannot tell you what your printer's dot gain is.

**Parity is what survives a lost sheet.** `--parity 0.2` protects against damage within
sheets. To survive losing a *whole* sheet you need roughly `--parity 1/(N-1)` where N is
the total sheet count: half for a three-sheet archive, a fifth for a six-sheet one.
`deckle estimate` prints the actual tolerance — *"any 1 of 3 sheets may be destroyed or
missing"* — and that line is the one to read. It covers fire, water, a torn page and an
envelope going missing, not just exotic failures.

**Ink.** Leave it at the default `--ink k`. Colour (`--ink cmy`) is built and it triples
capacity — three ink planes, three bits per cell — but it is deliberately **not rated for
long-term storage**, and for an archive that is the whole point. Colour inks and toners
fade unevenly and yellow goes first; black toner is fused carbon and is the most stable
printed material there is. Colour also needs a colour scan to read back, and the
reference decoder printed on the bootstrap page refuses colour pages by design, so a
colour archive depends on Deckle in a way a black one does not.

Colour earns its place when capacity matters more than decades: a large working set you
expect to re-print every few years, on pigment ink or colour toner, verified by an actual
round trip. If you cannot say when you will next re-print it, use black.

**Documentation.** Leave the bootstrap page on. `--no-bootstrap` is there for the case
where these sheets join a binder that already has one, or where paper is scarcer than the
tool — but an archive without it can only be read by Deckle, which is the thing the
project exists to avoid depending on.

## Storing it

- **Laser toner on acid-free paper.** Not dye inkjet: dye fades in years under light.
- **Two buildings, or one building and one trusted person.** Copies defeat more failure
  modes than any parameter does.
- **A fire safe or a dry box.** Paper's real enemies are fire, water, UV and insects,
  and none of those are addressed by error correction.
- **If you also store a scanner and a phone against the electronics being gone**, store
  them unpowered and shielded, and charge the phone once a year. Put a one-page
  procedure on the outside of the envelope so it can be read by torchlight without
  decoding anything: *scan at 600 dpi greyscale, sharpening off, then run
  `python3 dkl_ref.py scan-*.png -o recovered`.*
- **Verify once, now.** Decode your own scans and check the hash before you file it away.
  An archive nobody has ever read is a wish, not a backup.

## Limits, stated plainly

**Deckle does not encrypt anything yet.** The format reserves a field for it and
[PLAN.md](PLAN.md) §9.6 specifies `age`, but the tool does not implement it. **Encrypt
before you print.** A paper backup of a plaintext wallet seed or password database is a
bearer instrument: anyone who opens the envelope has it. Run it through `age`, `gpg` or
your password manager's own export encryption first, and make sure the passphrase is
recoverable by some other route — a printed key you cannot decrypt is the same as no key.

**This is not a disk replacement.** Tens to low hundreds of kilobytes per sheet. Anything
measured in gigabytes belongs on offline drives; paper holds the keys and the
configuration that let you use them.

**Phone cameras cannot read the dense grid.** Scanner for data sheets, phone for the
bootstrap page.

**No hardware measurement has been made.** Every capacity figure in this project comes
from a software loop. The printer-and-scanner testing that would confirm the density
tiers — Phase 0 in [PLAN.md](PLAN.md) §14 — has not been done. Treat `--cell 254` as the
safe default until you have done your own round trip, and read
[PROTOTYPE.md](PROTOTYPE.md) for what is actually built.

**Paper still burns.** Error correction handles a torn corner, a coffee ring, a crease
and a missing sheet. It does not handle the safe being in the fire. Copies, in separate
places, do.
