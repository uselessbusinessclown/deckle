# Use case: carrying data when devices get inspected

*The other use case is [long-term archival](USE-CASE-ARCHIVAL.md). Shared settings and
sizing are in [USE-CASES.md](USE-CASES.md).*

## The situation

Device inspection is routine at many borders and checkpoints, and in some jurisdictions
does not require suspicion. A laptop or phone may be examined, imaged, held for days, or
kept. Journalists, lawyers, aid workers, auditors and ordinary business travellers all
deal with this, and the standard advice is the same everywhere: **do not carry what you
cannot afford to have copied, and travel with as little on your devices as possible.**

That advice creates a gap. If the material must arrive with you and cannot go over the
network — because the network is monitored, unreliable, or because a cloud copy is itself
the thing you are avoiding — it has to travel somehow.

Printed pages travel differently from devices. There is no device to unlock, no account to
sign into, no cloud copy to subpoena, and nothing to image. A laptop can arrive wiped or
not arrive at all, and the data still reaches the destination.

## Be precise: this is opaque, not hidden

A Deckle sheet is **unreadable by inspection and completely obvious as data**. It is a
dense grid of black cells, and it looks like exactly what it is: something machine-readable.
It is not steganography, it does not disguise itself as anything else, and this
documentation offers no techniques for concealment.

What it gives you is a *different medium*, not invisibility:

| | a device | Deckle sheets |
|---|---|---|
| Unlock / credential to compel | yes | none exists |
| Can be imaged or cloned | yes, completely | yes, by photograph |
| Resists casual copying | somewhat | **no — a photo of the page is the data** |
| Cloud copy to subpoena | often | none |
| Arrives if the device does not | no | yes |
| Readable without your cooperation | with the credential | only the ciphertext |

Note the third row, because it cuts the other way and matters: **anyone who handles the
sheets can copy them completely and silently.** A phone photograph of a page is that page.
A locked device at least resists; paper does not resist at all. Everything therefore rests
on the encryption, not on the paper.

Paper is also inspectable, and what you must declare varies by jurisdiction and by what
you are carrying. Know your own obligations before you travel; this is a data format, not
legal advice, and using it to conceal something you are required to declare is your
problem and not a thing it solves.

## The workflow

```bash
# 1. Encrypt. Deckle does not do this for you.
age -R recipient.pub -o payload.age documents.tar

# 2. See what it costs before committing paper to it.
deckle estimate payload.age --cell 169 --ink cmy --parity 0.3

# 3. Print.
deckle encode payload.age --cell 169 --ink cmy --parity 0.3 --out sheets

# 4. At the destination: scan at 600 dpi greyscale (or colour, for colour ink),
#    sharpening off, then
deckle decode scan-*.png --out recovered
age -d -i recipient.key recovered/payload.age > documents.tar
```

## Encrypt first, and consider not holding the key

Deckle does not encrypt. [PLAN.md](PLAN.md) §9.6 specifies `age` and the format reserves a
field for it, but the tool does not implement it yet. **Encrypt before you encode.** An
unencrypted sheet is a bearer instrument that anyone who handles it can read at leisure.

For this use case specifically, prefer **encrypting to a recipient's public key** rather
than to a passphrase:

```bash
age -R destination.pub -o payload.age documents.tar
```

You then carry something you cannot decrypt yourself. In jurisdictions with
compelled-disclosure powers this is a materially different position from carrying a
passphrase you know, and it is standard practice among people who do this routinely. It
also means a lost or copied sheet is useless to whoever has it, without any action from
you.

If the material must be readable by you at the destination, generate the key pair there in
advance, or split the passphrase so that no single person or place holds it.

Encrypted data is **incompressible**, so the sheet counts below are exact rather than
pessimistic — Deckle detects this and skips compression rather than wasting your time.

## Settings for transport

Transport inverts the archival advice on ink, and this is the one place where full colour
is clearly the right choice.

**Ink: colour.** The objection to colour is that inks fade unevenly over years and yellow
goes first. That is decisive for a twenty-year archive and irrelevant to a document that
lives for a week. Use `--ink cmy` for three bits per cell and a third of the sheets — or
`--ink cm` if your printer's yellow is weak or you would rather not depend on the blue
channel of an unfamiliar scanner. The bootstrap page stays black either way.

**But check the scanner first.** A colour archive needs a *colour* scan to read back;
Deckle refuses a greyscale scan of a colour page by name rather than misreading it. If you
do not know what scanner will be available at the other end, print black — it reads on
anything.

**Cell size: 169 µm if you have tested your hardware, 254 µm if not.** Sheets that travel
in a bag get creased and rubbed. 254 µm survives noticeably more of that, and if you have
not done a round trip on the actual printer and scanner, the denser tier is a gamble taken
in the wrong place.

**Parity: 0.2 to 0.3 is plenty.** The archival case wants enough parity to survive losing
a whole sheet in storage over decades. Here the sheets travel together and arrive
together; parity is protecting against a crease, a coffee ring and a bad scan, not against
one envelope of six going missing. If the sheets will be split across couriers, raise it —
`deckle estimate` prints the real tolerance.

**Documentation: your call.** `--no-bootstrap` saves three sheets and is defensible when
you control both ends and will have Deckle at the destination. Leave it on if anyone else
might have to read the sheets, or if plans may change.

## Sizing

At 30% parity, on encrypted (therefore incompressible) input, so these are exact.

| Payload | 254 µm black | 169 µm black | 169 µm `cm` | 169 µm `cmy` |
|---|---|---|---|---|
| Keys, a credential set (32 KiB) | 1 | 1 | 1 | 1 |
| A working document set (256 KiB) | 6 | 3 | 2 | 1 |
| 1 MiB | 23 | 10 | 5 | 4 |
| 5 MiB | 113 | 50 | 25 | 17 |

Add three sheets for the bootstrap page unless you drop it.

A realistic ceiling is a few megabytes. Beyond that the stack of paper stops being
inconspicuous and starts being luggage — and at that point the honest answer is that
Deckle is the wrong tool and the material should go by some other route.

## Practical notes

- **Print with toner if you can.** Inkjet on plain paper smears when the bag gets damp.
  A laser page tolerates being folded, and colour toner tolerates it better than colour
  ink.
- **Do not fold along a column of cells if you can help it** — though the interleaving is
  designed for exactly this, and measured, a black page survives thirty-two fold lines.
- **Verify before you travel.** Scan your own printed sheets, decode them, check that the
  hash verifies and the reported correction margin says *healthy*. Do this on the actual
  printer you used. An untested stack of paper is worse than useless, because you will
  act as though the data arrived.
- **Know how you will read it back.** A flatbed scanner at 600 dpi. A phone camera will
  read the bootstrap page but **cannot** read the dense data grid — that is a scanner's
  job, and there is no way around it. Confirm one exists at the destination before you
  rely on this.
- **Destroy the sheets after use.** They do not stop being the data when you are done.
  Shred or burn; a page in a bin is a page in a bin.

## What this does not do

**It does not conceal anything.** See above. The sheets are visibly data.

**It does not protect against someone photographing the pages.** They do not need to
seize anything, and you will not know it happened. This is the central difference from a
device, and the reason the encryption is doing all the work.

**It does not resist coercion by itself.** If you can decrypt the payload, you can be
asked to. Encrypting to a key you do not hold is the answer to that, and it has to be
arranged before you leave.

**It does not survive being lost.** Unlike the archival case, there is usually no second
copy in another building. If the sheets are the only route, consider sending half by one
means and half by another, with parity high enough that either half plus the parity
sheets reconstructs the whole — `deckle estimate` will tell you what ratio that needs.
