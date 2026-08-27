#!/usr/bin/env python3
"""dkl_fec.py - rebuild missing Deckle blocks from parity, format 0x0100.

Only needed when pages are missing or damaged. If every page reads, dkl_ref.py
alone recovers the archive and you can ignore this file.

    python3 dkl_ref.py page-*.png --blocks blocks.bin     # writes what it read
    python3 dkl_fec.py blocks.bin -o recovered            # rebuilds the rest

Deckle's cross-block code is a systematic Reed-Solomon over GF(2^8) built on a
Cauchy generator matrix. Every square submatrix of a Cauchy matrix is
invertible, so ANY D surviving blocks of a group of D data blocks rebuild it,
whether they are data or parity. Standard library only.
"""

import sys, os, zlib, struct, hashlib

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


def cauchy(i, j, p):
    """Row i, column j of the generator matrix: 1 / (x_i + y_j), x and y disjoint."""
    return ginv((i & 0xFF) ^ ((p + j) & 0xFF))


def solve_group(data, parity, p, length):
    """Fill the None entries of `data` in place. Returns how many were rebuilt."""
    d = len(data)
    missing = [i for i in range(d) if data[i] is None]
    if not missing:
        return 0
    rows = [i for i in range(len(parity)) if parity[i] is not None][:len(missing)]
    if len(rows) < len(missing):
        raise SystemExit(
            "not enough parity: %d blocks missing, %d parity blocks survived"
            % (len(missing), len(rows)))

    # parity_i = sum_j A[i][j] * data_j. Move the known data terms to the right
    # hand side, leaving an m x m system in the blocks that are gone.
    m = len(missing)
    mat = [[0] * m for _ in range(m)]
    rhs = []
    for r, pi in enumerate(rows):
        acc = bytearray(parity[pi])
        for j in range(d):
            known = data[j]
            if known is not None:
                c = cauchy(pi, j, p)
                if c:
                    for k in range(length):
                        acc[k] ^= gmul(c, known[k])
        rhs.append(acc)
        for ci, j in enumerate(missing):
            mat[r][ci] = cauchy(pi, j, p)

    for col in range(m):
        piv = next((r for r in range(col, m) if mat[r][col]), None)
        if piv is None:
            raise SystemExit("parity matrix is singular; the block file may be damaged")
        mat[col], mat[piv] = mat[piv], mat[col]
        rhs[col], rhs[piv] = rhs[piv], rhs[col]
        inv = ginv(mat[col][col])
        mat[col] = [gmul(v, inv) for v in mat[col]]
        rhs[col] = bytearray(gmul(v, inv) for v in rhs[col])
        for r in range(m):
            if r != col and mat[r][col]:
                f = mat[r][col]
                src_m, src_r = mat[col], rhs[col]
                mat[r] = [a ^ gmul(f, b) for a, b in zip(mat[r], src_m)]
                dst = rhs[r]
                for k in range(length):
                    dst[k] ^= gmul(f, src_r[k])
    for ci, j in enumerate(missing):
        data[j] = bytes(rhs[ci])
    return m


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
    out = []
    for name, size, digest in meta:
        data = s[o:o + size]
        o += size
        if hashlib.sha256(data).digest() != digest:
            sys.stderr.write("WARNING: '%s' failed its own hash\n" % name)
        out.append((name, data))
    return out


def main(argv):
    path, outdir = None, "recovered"
    i = 0
    while i < len(argv):
        if argv[i] in ("-o", "--out"):
            i += 1
            outdir = argv[i]
        elif argv[i] in ("-h", "--help"):
            print(__doc__)
            return 0
        else:
            path = argv[i]
        i += 1
    if path is None:
        print(__doc__)
        return 2

    raw = open(path, 'rb').read()
    if raw[:4] != b'DKLB':
        sys.stderr.write("%s is not a block file from dkl_ref.py\n" % path)
        return 2
    (nd, ntotal, gd, gp, payload_len, bp, comp) = struct.unpack("<IIIIQBB", raw[4:30])
    sha_pre = raw[30:38]
    count = struct.unpack("<I", raw[38:42])[0]
    o = 42
    have = {}
    for _ in range(count):
        idx, _flags = struct.unpack("<IB", raw[o:o + 5])
        o += 5
        have[idx] = raw[o:o + bp]
        o += bp
    sys.stderr.write("%d of %d data blocks present, %d parity blocks present\n"
                     % (sum(1 for i in range(nd) if i in have), nd,
                        sum(1 for i in have if i >= nd)))

    groups = (nd + gd - 1) // gd if gd else 1
    rebuilt = 0
    data = [have.get(i) for i in range(nd)]
    for g in range(groups):
        s, e = g * gd, min((g + 1) * gd, nd)
        slice_ = data[s:e]
        if all(b is not None for b in slice_):
            continue
        if gp == 0:
            raise SystemExit("this archive was written without parity; the blocks are gone")
        parity = [have.get(nd + g * gp + j) for j in range(gp)]
        rebuilt += solve_group(slice_, parity, gp, bp)
        data[s:e] = slice_
    sys.stderr.write("rebuilt %d block(s) from parity\n" % rebuilt)

    payload = b''.join(data)[:payload_len]
    if comp == 1:
        payload = zlib.decompressobj(-15).decompress(payload)
    ok = hashlib.sha256(payload).digest()[:8] == sha_pre
    files = parse_manifest(payload)
    if files is None:
        sys.stderr.write("the rebuilt stream is not a Deckle manifest\n")
        return 1
    os.makedirs(outdir, exist_ok=True)
    for name, blob in files:
        p = os.path.join(outdir, os.path.basename(name))
        open(p, 'wb').write(blob)
        sys.stderr.write("wrote %s (%d bytes)\n" % (p, len(blob)))
    sys.stderr.write("document hash %s\n" % ("verified" if ok else "MISMATCH"))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
