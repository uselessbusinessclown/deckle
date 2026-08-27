#!/usr/bin/env python3
"""dkl_ref.py - reference decoder for Deckle paper archives, format 0x0100.

Reads scanned pages and reconstructs the original files. Written to be
re-implementable from docs/FORMAT.md alone, and to run on any Python 3 with
NOTHING but the standard library: no NumPy, no Pillow, no pip.

    python3 dkl_ref.py page-*.png -o recovered

Accepts 8-bit greyscale or RGB PNG, and binary PGM/PPM. Expect roughly half a
minute per A4 page at 600 dpi; this is a last-resort tool, not a fast one.

Reads black-only archives, which is what any archive meant to last should be.
Colour pages are refused with a message rather than misread.

If pages are missing or a page will not read, this decoder reports which blocks
are absent and stops. Reconstructing them from parity needs dkl_fec.py, which
is printed on the same bootstrap page.
"""

# SPDX-License-Identifier: MIT
# Copyright (c) 2026 the Deckle authors. Free to copy, use and republish;
# see the LICENSE file, or https://opensource.org/licenses/MIT

import sys, os, zlib, struct, hashlib, math

RS_N = 255
BLOCK_HEADER = 8
SYNC_PERIOD = 32
SYNC_BLOCK = 4
DESC_COLS, DESC_ROWS = 85, 24
DESC_BLOCK_COLS, DESC_BLOCK_ROWS = 91, 30
DESC_MARKER = 3
DESC_UNITS_ACROSS = 272.0
DESC_RS_K = 127
DESC_LEN = 96
WHITEN_SEED_DESC = 0x0DEC1E5C0DE5C001
WHITEN_SEED_DATA = 0x0DEC1E5DA7A00001
M64 = (1 << 64) - 1


# --------------------------------------------------------------- image input

def read_image(path):
    """Return (width, height, bytearray of 8-bit grey)."""
    raw = open(path, 'rb').read()
    if raw[:8] == b'\x89PNG\r\n\x1a\n':
        return _read_png(raw)
    if raw[:2] in (b'P5', b'P6'):
        return _read_pnm(raw)
    raise SystemExit("%s: not a PNG or binary PGM/PPM" % path)


def _read_pnm(raw):
    tok, pos = [], 2
    while len(tok) < 3:
        while pos < len(raw) and raw[pos:pos + 1].isspace():
            pos += 1
        if raw[pos:pos + 1] == b'#':
            while raw[pos:pos + 1] not in (b'\n', b''):
                pos += 1
            continue
        s = pos
        while pos < len(raw) and not raw[pos:pos + 1].isspace():
            pos += 1
        tok.append(int(raw[s:pos]))
    pos += 1
    w, h = tok[0], tok[1]
    body = raw[pos:]
    if raw[:2] == b'P5':
        return w, h, bytearray(body[:w * h])
    px = bytearray(w * h)
    for i in range(w * h):
        r, g, b = body[i * 3], body[i * 3 + 1], body[i * 3 + 2]
        px[i] = (r * 299 + g * 587 + b * 114) // 1000
    return w, h, px


def _read_png(raw):
    pos, idat, w, h, depth, ct = 8, [], 0, 0, 0, 0
    while pos < len(raw):
        ln = struct.unpack(">I", raw[pos:pos + 4])[0]
        typ = raw[pos + 4:pos + 8]
        if typ == b'IHDR':
            w, h, depth, ct = struct.unpack(">IIBB", raw[pos + 8:pos + 18])
            if raw[pos + 20] != 0:
                raise SystemExit("interlaced PNG not supported; re-save without Adam7")
        elif typ == b'IDAT':
            idat.append(raw[pos + 8:pos + 8 + ln])
        elif typ == b'IEND':
            break
        pos += 12 + ln
    if depth != 8 or ct not in (0, 2, 4, 6):
        raise SystemExit("need an 8-bit greyscale or RGB PNG (got depth %d, colour %d)"
                         % (depth, ct))
    nch = {0: 1, 2: 3, 4: 2, 6: 4}[ct]
    buf = zlib.decompress(b''.join(idat))
    stride = w * nch
    out = bytearray(stride * h)
    prev = bytearray(stride)
    o = 0
    for y in range(h):
        p = y * (stride + 1)
        f = buf[p]
        row = buf[p + 1:p + 1 + stride]
        if f == 0:
            cur = bytearray(row)
        elif f == 1:
            cur = bytearray(stride)
            for i in range(stride):
                cur[i] = (row[i] + (cur[i - nch] if i >= nch else 0)) & 255
        elif f == 2:
            cur = bytearray(stride)
            for i in range(stride):
                cur[i] = (row[i] + prev[i]) & 255
        elif f == 3:
            cur = bytearray(stride)
            for i in range(stride):
                a = cur[i - nch] if i >= nch else 0
                cur[i] = (row[i] + ((a + prev[i]) >> 1)) & 255
        else:
            cur = bytearray(stride)
            for i in range(stride):
                a = cur[i - nch] if i >= nch else 0
                c = prev[i - nch] if i >= nch else 0
                b = prev[i]
                pp = a + b - c
                pa, pb, pc = abs(pp - a), abs(pp - b), abs(pp - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                cur[i] = (row[i] + pr) & 255
        out[o:o + stride] = cur
        o += stride
        prev = cur
    if nch == 1:
        return w, h, out
    grey = bytearray(w * h)
    for i in range(w * h):
        r, g, b = out[i * nch], out[i * nch + 1], out[i * nch + 2]
        grey[i] = (r * 299 + g * 587 + b * 114) // 1000
    return w, h, grey


# ------------------------------------------------------- GF(2^8), Reed-Solomon

GF_EXP = [0] * 512
GF_LOG = [0] * 256
_x = 1
for _i in range(255):
    GF_EXP[_i] = _x
    GF_LOG[_x] = _i
    _x <<= 1
    if _x & 0x100:
        _x ^= 0x11D
for _i in range(255, 512):
    GF_EXP[_i] = GF_EXP[_i - 255]


def gmul(a, b):
    return 0 if a == 0 or b == 0 else GF_EXP[GF_LOG[a] + GF_LOG[b]]


def ginv(a):
    return GF_EXP[255 - GF_LOG[a]]


def gpow(a, e):
    return 0 if a == 0 else GF_EXP[(GF_LOG[a] * e) % 255]


def poly_mul(a, b):
    out = [0] * (len(a) + len(b) - 1)
    for i, x in enumerate(a):
        if x:
            for j, y in enumerate(b):
                out[i + j] ^= gmul(x, y)
    return out


def poly_eval(p, x):
    acc = 0
    for c in reversed(p):
        acc = gmul(acc, x) ^ c
    return acc


def rs_decode(cw, nsym):
    """Correct up to nsym//2 symbol errors in place. True on success."""
    n = len(cw)
    synd = []
    for i in range(nsym):
        x = gpow(2, i)
        acc = 0
        for c in cw:
            acc = gmul(acc, x) ^ c
        synd.append(acc)
    if not any(synd):
        return True
    err_loc, old_loc = [1], [1]
    for i in range(nsym):
        delta = synd[i]
        for j in range(1, len(err_loc)):
            if j <= i:
                delta ^= gmul(err_loc[j], synd[i - j])
        old_loc = [0] + old_loc
        if delta:
            if len(old_loc) > len(err_loc):
                new_loc = [gmul(c, delta) for c in old_loc]
                d_inv = ginv(delta)
                old_loc = [gmul(c, d_inv) for c in err_loc]
                err_loc = new_loc
            scaled = [gmul(c, delta) for c in old_loc]
            if len(scaled) > len(err_loc):
                err_loc = err_loc + [0] * (len(scaled) - len(err_loc))
            for k, v in enumerate(scaled):
                err_loc[k] ^= v
    while len(err_loc) > 1 and err_loc[-1] == 0:
        err_loc.pop()
    nerr = len(err_loc) - 1
    if 2 * nerr > nsym:
        return False
    pos = [p for p in range(n) if poly_eval(err_loc, ginv(gpow(2, n - 1 - p))) == 0]
    if len(pos) != nerr:
        return False
    xs = [gpow(2, n - 1 - p) for p in pos]
    lam = [1]
    for x in xs:
        lam = poly_mul(lam, [1, x])
    omega = poly_mul(synd, lam)[:nsym]
    for i, p in enumerate(pos):
        xi_inv = ginv(xs[i])
        prime = 1
        for j, xj in enumerate(xs):
            if j != i:
                prime = gmul(prime, 1 ^ gmul(xi_inv, xj))
        if prime == 0:
            return False
        mag = gmul(poly_eval(omega, xi_inv), ginv(prime))
        cw[p] ^= mag
    for i in range(nsym):
        x = gpow(2, i)
        acc = 0
        for c in cw:
            acc = gmul(acc, x) ^ c
        if acc:
            return False
    return True


_CRCT = []
for _i in range(256):
    _c = _i
    for _ in range(8):
        _c = (_c >> 1) ^ (0x82F63B78 if _c & 1 else 0)
    _CRCT.append(_c)


def crc32c(data):
    c = 0xFFFFFFFF
    for b in data:
        c = _CRCT[(c ^ b) & 0xFF] ^ (c >> 8)
    return c ^ 0xFFFFFFFF


# ----------------------------------------------------------------- geometry

def homography(src, dst):
    """3x3 taking src[i] -> dst[i] for four correspondences, as a 9-list."""
    a = []
    for i in range(4):
        x, y = src[i]
        u, v = dst[i]
        a.append([x, y, 1, 0, 0, 0, -u * x, -u * y, u])
        a.append([0, 0, 0, x, y, 1, -v * x, -v * y, v])
    for col in range(8):
        piv = max(range(col, 8), key=lambda r: abs(a[r][col]))
        if abs(a[piv][col]) < 1e-12:
            return None
        a[col], a[piv] = a[piv], a[col]
        d = a[col][col]
        a[col] = [v / d for v in a[col]]
        for r in range(8):
            if r != col and a[r][col]:
                f = a[r][col]
                a[r] = [v - f * w for v, w in zip(a[r], a[col])]
    return [a[i][8] for i in range(8)] + [1.0]


def happly(m, x, y):
    d = m[6] * x + m[7] * y + m[8]
    return ((m[0] * x + m[1] * y + m[2]) / d, (m[3] * x + m[4] * y + m[5]) / d)


# --------------------------------------------- layout, mirrored from the spec

def in_corner(x, y, cols, rows, f):
    return (x < f or x >= cols - f) and (y < f or y >= rows - f)


def is_reserved(cols, rows, f, x, y):
    if in_corner(x, y, cols, rows, f):
        return True
    if x % SYNC_PERIOD < SYNC_BLOCK and y % SYNC_PERIOD < SYNC_BLOCK:
        bx, by = x - x % SYNC_PERIOD, y - y % SYNC_PERIOD
        return (bx + SYNC_BLOCK <= cols and by + SYNC_BLOCK <= rows
                and not in_corner(bx, by, cols, rows, f)
                and not in_corner(bx + SYNC_BLOCK - 1, by + SYNC_BLOCK - 1, cols, rows, f))
    return False


def sync_marks(cols, rows, f):
    out = []
    sy = 0
    while sy + SYNC_BLOCK <= rows:
        sx = 0
        while sx + SYNC_BLOCK <= cols:
            if (not in_corner(sx, sy, cols, rows, f)
                    and not in_corner(sx + SYNC_BLOCK - 1, sy + SYNC_BLOCK - 1, cols, rows, f)):
                out.append((sx, sy))
            sx += SYNC_PERIOD
        sy += SYNC_PERIOD
    return out


def bands_of(cols, rows, f, band_rows):
    """[(row0, row1, cells, codewords, first_cw)]"""
    out, first, r = [], 0, 0
    while r < rows:
        r1 = min(r + band_rows, rows)
        cells = sum(1 for y in range(r, r1) for x in range(cols)
                    if not is_reserved(cols, rows, f, x, y))
        cw = cells // (RS_N * 8)
        out.append((r, r1, cells, cw, first))
        first += cw
        r = r1
    return out


def choose_a(c):
    if c < 3:
        return 1
    a = min(max(int(math.floor(c * 0.618033988749895 + 0.5)), 2), c - 1)
    for _ in range(c):
        if math.gcd(a, c) == 1:
            return a
        a += 1
        if a >= c:
            a = 2
    return 1


def desc_gap_cells(ratio):
    return 4.5 * ratio + 2.0


class Whitener:
    """xorshift64* keystream, one bit per cell."""

    def __init__(self, seed):
        self.s = (seed ^ 0x9E3779B97F4A7C15) & M64
        self.buf = 0
        self.left = 0

    def bit(self):
        if self.left == 0:
            x = self.s
            x ^= (x << 13) & M64
            x ^= x >> 7
            x ^= (x << 17) & M64
            self.s = x
            self.buf = (x * 0x2545F4914F6CDD1D) & M64
            self.left = 64
        b = self.buf & 1
        self.buf >>= 1
        self.left -= 1
        return b


# ------------------------------------------------------------- image helpers

class Img:
    def __init__(self, w, h, px):
        self.w, self.h, self.px = w, h, px

    def at(self, x, y):
        if x < 0:
            x = 0
        elif x >= self.w:
            x = self.w - 1
        if y < 0:
            y = 0
        elif y >= self.h:
            y = self.h - 1
        return self.px[y * self.w + x]

    def sample(self, fx, fy):
        x0, y0 = int(fx), int(fy)
        dx, dy = fx - x0, fy - y0
        a = self.at(x0, y0)
        b = self.at(x0 + 1, y0)
        c = self.at(x0, y0 + 1)
        d = self.at(x0 + 1, y0 + 1)
        return (a * (1 - dx) + b * dx) * (1 - dy) + (c * (1 - dx) + d * dx) * dy


def otsu(img):
    hist = [0] * 256
    for v in img.px:
        hist[v] += 1
    total = len(img.px)
    s = sum(i * hist[i] for i in range(256))
    wb, sb, best, thr = 0, 0.0, -1.0, 128
    for t in range(256):
        wb += hist[t]
        if wb == 0:
            continue
        wf = total - wb
        if wf == 0:
            break
        sb += t * hist[t]
        v = wb * wf * (sb / wb - (s - sb) / wf) ** 2
        if v > best:
            best, thr = v, t
    return thr


class Threshold:
    """Local midpoint threshold on a coarse tile grid.

    A page of black and white cells has both extremes inside any tile a few
    cells across, so (min+max)/2 per tile tracks an illumination gradient
    without a full Sauvola pass, which pure Python cannot afford.
    """

    TILE = 16

    def __init__(self, img, stride=2):
        t = self.TILE
        self.tw = (img.w + t - 1) // t
        self.th = (img.h + t - 1) // t
        lo = [255] * (self.tw * self.th)
        hi = [0] * (self.tw * self.th)
        for y in range(0, img.h, stride):
            base = y * img.w
            ty = (y // t) * self.tw
            for x in range(0, img.w, stride):
                v = img.px[base + x]
                i = ty + x // t
                if v < lo[i]:
                    lo[i] = v
                if v > hi[i]:
                    hi[i] = v
        self.mid = [0.0] * (self.tw * self.th)
        for i in range(len(lo)):
            self.mid[i] = (lo[i] + hi[i]) * 0.5 if hi[i] > lo[i] else 128.0
        self.contrast = [max(hi[i] - lo[i], 1) for i in range(len(lo))]

    def at(self, px, py):
        t = self.TILE
        gx = px / t - 0.5
        gy = py / t - 0.5
        x0 = min(max(int(math.floor(gx)), 0), self.tw - 1)
        y0 = min(max(int(math.floor(gy)), 0), self.th - 1)
        x1 = min(x0 + 1, self.tw - 1)
        y1 = min(y0 + 1, self.th - 1)
        fx = min(max(gx - x0, 0.0), 1.0)
        fy = min(max(gy - y0, 0.0), 1.0)
        m = self.mid
        return ((m[y0 * self.tw + x0] * (1 - fx) + m[y0 * self.tw + x1] * fx) * (1 - fy)
                + (m[y1 * self.tw + x0] * (1 - fx) + m[y1 * self.tw + x1] * fx) * fy)


def refine_dark(img, cx, cy, radius):
    """Darkness-weighted centroid in a window; None if it has no contrast."""
    r = max(radius, 1.0)
    x0, y0 = max(int(cx - r), 0), max(int(cy - r), 0)
    x1, y1 = min(int(cx + r) + 1, img.w), min(int(cy + r) + 1, img.h)
    if x1 <= x0 or y1 <= y0:
        return None
    lo, hi = 255, 0
    for y in range(y0, y1):
        base = y * img.w
        for x in range(x0, x1):
            v = img.px[base + x]
            if v < lo:
                lo = v
            if v > hi:
                hi = v
    if hi - lo < 24:
        return None
    cut = (lo + hi) * 0.5
    sx = sy = sw = 0.0
    for y in range(y0, y1):
        base = y * img.w
        for x in range(x0, x1):
            v = img.px[base + x]
            if v < cut:
                w = cut - v
                sx += x * w
                sy += y * w
                sw += w
    if sw <= 0:
        return None
    return (sx / sw + 0.5, sy / sw + 0.5)


# ------------------------------------------------------------ page location

def verify_vertical(img, thr, x, y, m):
    if x >= img.w or img.at(x, y) > thr:
        return None
    def runs(step):
        out, pos, colour = [], y, True
        for _ in range(3):
            n = 0
            while 0 <= pos + step < img.h and (img.at(x, pos + step) <= thr) == colour:
                pos += step
                n += 1
                if n > m * 6:
                    return None
            out.append(n)
            colour = not colour
        return out
    up, dn = runs(-1), runs(1)
    if up is None or dn is None:
        return None
    if abs((up[0] + dn[0] + 1) - 3 * m) > 2 * m:
        return None
    for k in (1, 2):
        if abs(up[k] - m) > m * 0.8 or abs(dn[k] - m) > m * 0.8:
            return None
    return y + (dn[0] - up[0]) / 2.0


def find_finders(img, thr):
    """Locate finder patterns by their 1:1:3:1:1 run signature."""
    cands = []
    for y in range(0, img.h, 2):
        base = y * img.w
        runs = []
        cur = img.px[base] <= thr
        start = 0
        for x in range(1, img.w + 1):
            d = (img.px[base + x] <= thr) if x < img.w else (not cur)
            if d != cur:
                runs.append((cur, start, x - start))
                cur, start = d, x
        for i in range(len(runs) - 4):
            w = runs[i:i + 5]
            if not (w[0][0] and not w[1][0] and w[2][0] and not w[3][0] and w[4][0]):
                continue
            total = sum(r[2] for r in w)
            m = total / 7.0
            if m < 1.2:
                continue
            v = m * 0.6
            if not (abs(w[0][2] - m) < v and abs(w[1][2] - m) < v
                    and abs(w[2][2] - 3 * m) < 3 * v
                    and abs(w[3][2] - m) < v and abs(w[4][2] - m) < v):
                continue
            cx = w[2][1] + w[2][2] / 2.0
            cy = verify_vertical(img, thr, int(cx + 0.5), y, m)
            if cy is not None:
                cands.append((cx, cy, m))
    clusters = []
    for cx, cy, m in cands:
        for c in clusters:
            if abs(c[0] / c[3] - cx) < m * 2 and abs(c[1] / c[3] - cy) < m * 2:
                c[0] += cx; c[1] += cy; c[2] += m; c[3] += 1
                break
        else:
            clusters.append([cx, cy, m, 1])
    out = [(c[0] / c[3], c[1] / c[3], c[2] / c[3], c[3]) for c in clusters if c[3] >= 3]
    if not out:
        return []
    out.sort(key=lambda c: -c[3])
    top, unit0 = out[0][3], out[0][2]
    out = [c for c in out if c[3] >= max(top * 0.25, 3) and abs(c[2] - unit0) / unit0 < 0.25]
    return [(c[0], c[1], c[2]) for c in out[:12]]


def orient_candidates(f):
    """Plausible (TL, TR, BL, unit) triples, largest right-angled corner first."""
    scored = []
    n = len(f)
    for i in range(n):
        for j in range(n):
            for k in range(j + 1, n):
                if i in (j, k):
                    continue
                ax, ay = f[i][0], f[i][1]
                v1 = (f[j][0] - ax, f[j][1] - ay)
                v2 = (f[k][0] - ax, f[k][1] - ay)
                l1 = math.hypot(*v1)
                l2 = math.hypot(*v2)
                if l1 < 1 or l2 < 1:
                    continue
                if abs(v1[0] * v2[0] + v1[1] * v2[1]) / (l1 * l2) > 0.18:
                    continue
                scored.append((abs(v1[0] * v2[1] - v1[1] * v2[0]), i, j, k))
    scored.sort(key=lambda s: -s[0])
    out = []
    for _, i, j, k in scored[:24]:
        tl = (f[i][0], f[i][1])
        tr, bl = (f[j][0], f[j][1]), (f[k][0], f[k][1])
        if (tr[0] - tl[0]) * (bl[1] - tl[1]) - (tr[1] - tl[1]) * (bl[0] - tl[0]) < 0:
            tr, bl = bl, tr
        out.append((tl, tr, bl, (f[i][2] + f[j][2] + f[k][2]) / 3.0))
    return out


def box_mean(img, cx, cy, half):
    x0, y0 = max(int(cx - half), 0), max(int(cy - half), 0)
    x1, y1 = min(int(cx + half) + 1, img.w), min(int(cy + half) + 1, img.h)
    if x1 <= x0 or y1 <= y0:
        return 255.0
    s = n = 0
    for y in range(y0, y1):
        base = y * img.w
        for x in range(x0, x1):
            s += img.px[base + x]
            n += 1
    return s / n


def locate_br(img, cx, cy, unit):
    """Find the solid bottom-right marker near a predicted position.

    The prediction TR + BL - TL is a parallelogram, which a homography does not
    preserve, so search a neighbourhood for the darkest box and verify it
    against the marker's white ring. Both tests are relative to local
    brightness so an illumination gradient cannot veto a good marker.
    """
    half = max(unit * 1.5, 2.0)
    search = max(unit * 8.0, 8.0)
    step = max(unit * 0.35, 1.0)
    best = None
    dy = -search
    while dy <= search:
        dx = -search
        while dx <= search:
            px, py = cx + dx, cy + dy
            if half <= px < img.w - half and half <= py < img.h - half:
                m = box_mean(img, px, py, half)
                if best is None or m < best[0]:
                    best = (m, px, py)
            dx += step
        dy += step
    if best is None:
        return None
    core, px, py = best
    a = unit * 3.5
    pts, s = [], 0.0
    for k in range(5):
        t = unit * (k - 2)
        pts += [(a, t), (-a, t), (t, a), (t, -a)]
    for dx, dy in pts:
        x, y = px + dx, py + dy
        if not (0 <= x < img.w and 0 <= y < img.h):
            return None
        s += img.sample(x, y)
    ring = s / len(pts)
    if ring - core < 90 or core > ring * 0.45:
        return None
    r = refine_dark(img, px, py, unit * 3.0)
    return r if r else (px, py)


# ------------------------------------------------------------- descriptor

def parse_descriptor(m):
    if len(m) < DESC_LEN + 4 or m[:4] != b'DKLP':
        return None
    if crc32c(m[:DESC_LEN]) != struct.unpack("<I", m[DESC_LEN:DESC_LEN + 4])[0]:
        return None
    u = lambda o, n: int.from_bytes(m[o:o + n], 'little')
    return dict(
        format_version=u(4, 2), symbology=u(6, 2), uuid=bytes(m[8:24]),
        sha_pre=bytes(m[24:32]), page=u(32, 2), pages=u(34, 2), cell_um=u(36, 2),
        cols=u(38, 2), rows=u(40, 2), sync=m[42], fid=m[43], seed=u(44, 4),
        rs_n=m[52], rs_k=m[53], block_payload=m[54], seq_start=u(55, 4),
        blocks=u(59, 2), compression=m[61], encryption=m[62], fec=m[63],
        fec_data=u(64, 4), fec_parity=u(68, 4), total_data=u(72, 4),
        total_blocks=u(76, 4), payload_len=u(80, 4), ink_planes=m[84],
        cal_period=m[85], cal_patch_cells=m[86], plane_reg_spec=m[87],
        dpi=u(88, 2), provenance=m[90], flags=m[91], band_rows=u(92, 2))


def read_descriptor(img, thr, h, aspect, span_x, unit_px):
    du = 1.0 / DESC_UNITS_ACROSS
    dv = aspect / DESC_UNITS_ACROSS
    ds_px = du * span_x
    top = -(DESC_BLOCK_ROWS + desc_gap_cells(unit_px / ds_px)) * dv
    mk = DESC_MARKER / 2.0
    nominal = [(mk, mk), (DESC_BLOCK_COLS - mk, mk),
               (mk, DESC_BLOCK_ROWS - mk), (DESC_BLOCK_COLS - mk, DESC_BLOCK_ROWS - mk)]
    dst = []
    for cx, cy in nominal:
        px, py = happly(h, cx * du, top + cy * dv)
        r = refine_dark(img, px, py, ds_px * 2.5)
        dst.append(r if r else (px, py))
    hs = homography(nominal, dst)
    if hs is None:
        return None
    cw = bytearray(RS_N)
    wh = Whitener(WHITEN_SEED_DESC)
    for r in range(DESC_ROWS):
        for c in range(DESC_COLS):
            px, py = happly(hs, c + DESC_MARKER + 0.5, r + DESC_MARKER + 0.5)
            bit = (1 if img.sample(px, py) < thr.at(px, py) else 0) ^ wh.bit()
            if bit:
                i = r * DESC_COLS + c
                cw[i >> 3] |= 1 << (7 - (i & 7))
    if not rs_decode(cw, RS_N - DESC_RS_K):
        return None
    return parse_descriptor(cw[:DESC_RS_K])


# ------------------------------------------------------------- page decoding

def mirror(img):
    out = bytearray(len(img.px))
    for y in range(img.h):
        base = y * img.w
        row = img.px[base:base + img.w]
        row.reverse()
        out[base:base + img.w] = row
    return Img(img.w, img.h, out)


def decode_page(img, verbose=False):
    """Return (descriptor, {block_index: payload}) or (None, reason)."""
    for flipped in (False, True):
        cur = mirror(img) if flipped else img
        thr_g = otsu(cur)
        finders = find_finders(cur, thr_g)
        if len(finders) < 3:
            continue
        thr = Threshold(cur)
        for tl, tr, bl, unit in orient_candidates(finders):
            br = locate_br(cur, tr[0] + bl[0] - tl[0], tr[1] + bl[1] - tl[1], unit)
            if br is None:
                continue
            h = homography([(0, 0), (1, 0), (0, 1), (1, 1)], [tl, tr, bl, br])
            if h is None:
                continue
            span_x = math.dist(tl, tr)
            span_y = math.dist(tl, bl)
            aspect = span_x / span_y if span_y > 0 else 1.0
            d = read_descriptor(cur, thr, h, aspect, span_x, unit)
            if d is None:
                continue
            if d['ink_planes'] != 0:
                return None, (
                    "this page is in colour mode (three ink planes), which this\n"
                    "  decoder does not read. Colour archives are not rated for\n"
                    "  long-term storage for exactly this reason - use deckle, or\n"
                    "  re-print the archive in black only.")
            if verbose:
                sys.stderr.write("  page %d/%d, %dx%d cells, RS(255,%d)%s\n"
                                 % (d['page'] + 1, d['pages'], d['cols'], d['rows'],
                                    d['rs_k'], ", mirrored" if flipped else ""))
            return d, sample_blocks(cur, thr, h, d, span_x)
    return None, "no readable descriptor in either orientation"


def sample_blocks(img, thr, h, d, span_x):
    cols, rows, f = d['cols'], d['rows'], d['fid']
    nsym = RS_N - d['rs_k']
    cell_px = span_x / (cols - f)
    half_f = f / 2.0
    wu = float(cols - f)
    hv = float(rows - f)

    # Local warp: measure each sync mark and interpolate the displacement between.
    nx = (cols + SYNC_PERIOD - 1) // SYNC_PERIOD
    ny = (rows + SYNC_PERIOD - 1) // SYNC_PERIOD
    disp = [None] * (nx * ny)
    for bx, by in sync_marks(cols, rows, f):
        px, py = happly(h, (bx + 2 - half_f) / wu, (by + 2 - half_f) / hv)
        r = refine_dark(img, px, py, cell_px * 1.1)
        if r:
            dx, dy = r[0] - px, r[1] - py
            if math.hypot(dx, dy) / cell_px < 1.5:
                disp[(by // SYNC_PERIOD) * nx + bx // SYNC_PERIOD] = (dx, dy)

    def warp_at(cx, cy):
        gx = min(max(cx / SYNC_PERIOD, 0.0), nx - 1.0)
        gy = min(max(cy / SYNC_PERIOD, 0.0), ny - 1.0)
        x0, y0 = int(gx), int(gy)
        x1, y1 = min(x0 + 1, nx - 1), min(y0 + 1, ny - 1)
        fx, fy = gx - x0, gy - y0
        ax = ay = w = 0.0
        for xi, yi, ww in ((x0, y0, (1 - fx) * (1 - fy)), (x1, y0, fx * (1 - fy)),
                           (x0, y1, (1 - fx) * fy), (x1, y1, fx * fy)):
            v = disp[yi * nx + xi]
            if v:
                ax += v[0] * ww
                ay += v[1] * ww
                w += ww
        return (ax / w, ay / w) if w > 1e-6 else (0.0, 0.0)

    bands = bands_of(cols, rows, f, d['band_rows'] or 128)
    n = sum(b[3] for b in bands)
    cws = [bytearray(RS_N) for _ in range(n)]
    wh = Whitener(WHITEN_SEED_DATA ^ d['page'])
    for bi, (r0, r1, cells, ncw, first) in enumerate(bands):
        a = choose_a(cells)
        b = (d['seed'] + bi * 7919) % max(cells, 1)
        used = ncw * RS_N * 8
        p = 0
        for y in range(r0, r1):
            vy = (y + 0.5 - half_f) / hv
            for x in range(cols):
                if is_reserved(cols, rows, f, x, y):
                    continue
                pp = (a * p + b) % cells
                white = wh.bit()
                p += 1
                if pp >= used or ncw == 0:
                    continue
                px, py = happly(h, (x + 0.5 - half_f) / wu, vy)
                dx, dy = warp_at(x, y)
                px += dx
                py += dy
                if (1 if img.sample(px, py) < thr.at(px, py) else 0) ^ white:
                    cw = first + pp % ncw
                    idx = pp // ncw
                    cws[cw][idx >> 3] |= 1 << (7 - (idx & 7))

    out, bad = {}, 0
    k = d['rs_k']
    for cw in cws:
        if not rs_decode(cw, nsym):
            bad += 1
            continue
        index = int.from_bytes(cw[0:3], 'little')
        flags = cw[3]
        payload = bytes(cw[BLOCK_HEADER:k])
        if crc32c(bytes(cw[0:4]) + payload) != int.from_bytes(cw[4:8], 'little'):
            bad += 1
            continue
        if flags & 0x02 or index == 0x00FFFFFF:
            continue          # page filler, not payload
        out[index] = (payload, flags)
    if bad:
        sys.stderr.write("  %d of %d codewords on this page could not be read\n"
                         % (bad, len(cws)))
    return out


# ---------------------------------------------------------------- assembly

def parse_manifest(s):
    if len(s) < 32 or s[:4] != b'DKL1':
        return None
    o = 30
    n = int.from_bytes(s[o:o + 2], 'little')
    o += 2
    meta = []
    for _ in range(n):
        nl = int.from_bytes(s[o:o + 2], 'little')
        o += 2
        name = s[o:o + nl].decode('utf-8', 'replace')
        o += nl
        size = int.from_bytes(s[o:o + 8], 'little')
        o += 8
        digest = s[o:o + 32]
        o += 32
        meta.append((name, size, digest))
    files = []
    for name, size, digest in meta:
        data = s[o:o + size]
        o += size
        if hashlib.sha256(data).digest() != digest:
            sys.stderr.write("  WARNING: '%s' failed its own hash\n" % name)
        files.append((name, data))
    return files


def main(argv):
    paths, outdir, blocks_out = [], "recovered", None
    i = 0
    while i < len(argv):
        if argv[i] in ("-o", "--out"):
            i += 1
            outdir = argv[i]
        elif argv[i] == "--blocks":
            i += 1
            blocks_out = argv[i]
        elif argv[i] in ("-h", "--help"):
            print(__doc__)
            return 0
        else:
            paths.append(argv[i])
        i += 1
    if not paths:
        print(__doc__)
        return 2

    desc, blocks = None, {}
    for path in paths:
        sys.stderr.write("%s\n" % os.path.basename(path))
        w, h, px = read_image(path)
        d, res = decode_page(Img(w, h, px), verbose=True)
        if d is None:
            sys.stderr.write("  FAILED: %s\n" % res)
            continue
        if desc is None:
            desc = d
        elif d['uuid'] != desc['uuid']:
            sys.stderr.write("  SKIPPED: belongs to a different document\n")
            continue
        blocks.update(res)
    if desc is None:
        sys.stderr.write("no page could be read\n")
        return 1

    nd = desc['total_data']
    missing = [i for i in range(nd) if i not in blocks]
    if missing:
        sys.stderr.write(
            "\n%d of %d data blocks are missing.\n"
            "Rescan the pages that failed. If pages are lost for good, the parity\n"
            "blocks can rebuild them - run dkl_fec.py, also printed on the\n"
            "bootstrap page. Writing what was read to '%s'.\n"
            % (len(missing), nd, blocks_out or "blocks.bin"))
        write_blocks(blocks_out or "blocks.bin", desc, blocks)
        return 1

    payload = b''.join(blocks[i][0] for i in range(nd))[:desc['payload_len']]
    if desc['compression'] == 1:
        payload = zlib.decompressobj(-15).decompress(payload)
    elif desc['compression'] != 0:
        sys.stderr.write("unknown compression id %d\n" % desc['compression'])
        return 1
    if desc['encryption'] != 0:
        sys.stderr.write("this archive is encrypted; decrypt the stream after recovery\n")
        return 1

    digest = hashlib.sha256(payload).digest()
    ok = digest[:8] == desc['sha_pre']
    files = parse_manifest(payload)
    if files is None:
        sys.stderr.write("the recovered stream is not a Deckle manifest\n")
        return 1
    os.makedirs(outdir, exist_ok=True)
    for name, data in files:
        p = os.path.join(outdir, os.path.basename(name))
        open(p, 'wb').write(data)
        sys.stderr.write("wrote %s (%d bytes)\n" % (p, len(data)))
    sys.stderr.write("document hash %s\n" % ("verified" if ok else "MISMATCH"))
    return 0 if ok else 1


def write_blocks(path, desc, blocks):
    """Partial recovery: index, flags and payload per block, for dkl_fec.py."""
    with open(path, 'wb') as fh:
        fh.write(b'DKLB')
        fh.write(struct.pack("<IIIIQBB", desc['total_data'], desc['total_blocks'],
                             desc['fec_data'], desc['fec_parity'], desc['payload_len'],
                             desc['block_payload'], desc['compression']))
        fh.write(desc['sha_pre'])
        fh.write(struct.pack("<I", len(blocks)))
        for idx in sorted(blocks):
            payload, flags = blocks[idx]
            fh.write(struct.pack("<IB", idx, flags))
            fh.write(payload)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
