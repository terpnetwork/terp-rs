#!/usr/bin/env python3
"""Recompute a CosmWasm instantiate2 address (wasmd BuildContractAddressPredictable, empty init msg).
usage: instantiate2_check.py <code_checksum_hex> <creator_bech32> <salt_base64> [prefix]"""
import base64, hashlib, sys

CH = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
def _polymod(v):
    g = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]; c = 1
    for x in v:
        b = c >> 25; c = (c & 0x1ffffff) << 5 ^ x
        for i in range(5): c ^= g[i] if (b >> i) & 1 else 0
    return c
def _hrp(h): return [ord(x) >> 5 for x in h] + [0] + [ord(x) & 31 for x in h]
def _bits(d, f, t, pad):
    acc = bits = 0; out = []; m = (1 << t) - 1
    for v in d:
        acc = (acc << f) | v; bits += f
        while bits >= t: bits -= t; out.append((acc >> bits) & m)
    if pad and bits: out.append((acc << (t - bits)) & m)
    return out
def b32decode(a):
    h, d = a.rsplit("1", 1); v = [CH.index(c) for c in d]
    assert _polymod(_hrp(h) + v) == 1, "bad checksum"
    return h, bytes(_bits(v[:-6], 5, 8, False))
def b32encode(h, data):
    v = _bits(data, 8, 5, True); p = _polymod(_hrp(h) + v + [0] * 6) ^ 1
    return h + "1" + "".join(CH[x] for x in v + [(p >> 5 * (5 - i)) & 31 for i in range(6)])

def lp(b): return len(b).to_bytes(8, "big") + b
def instantiate2(checksum, creator, salt, msg=b""):
    key = b"wasm\x00" + lp(checksum) + lp(creator) + lp(salt) + lp(msg)
    return hashlib.sha256(hashlib.sha256(b"module").digest() + key).digest()

if __name__ == "__main__":
    cs, creator, salt = sys.argv[1:4]
    hrp, cb = b32decode(creator)
    print(b32encode(sys.argv[4] if len(sys.argv) > 4 else hrp, instantiate2(bytes.fromhex(cs), cb, base64.b64decode(salt))))
