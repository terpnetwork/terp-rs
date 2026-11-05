#!/usr/bin/env python3
"""ADR-36 signature for a headstash operator route. Prints the compact hex signature only."""

import argparse
import hashlib
import sys

from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import (
    Prehashed,
    decode_dss_signature,
)
from cryptography.hazmat.primitives import hashes

CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
SECP_N = int(
    "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141", 16
)


def bech32_polymod(values):
    generator = [0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3]
    chk = 1
    for value in values:
        top = chk >> 25
        chk = ((chk & 0x1FFFFFF) << 5) ^ value
        for i in range(5):
            if (top >> i) & 1:
                chk ^= generator[i]
    return chk


def bech32_hrp_expand(hrp):
    return [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp]


def bech32_create_checksum(hrp, data):
    values = bech32_hrp_expand(hrp) + data
    polymod = bech32_polymod(values + [0, 0, 0, 0, 0, 0]) ^ 1
    return [(polymod >> 5 * (5 - i)) & 31 for i in range(6)]


def bech32_encode(hrp, data):
    combined = data + bech32_create_checksum(hrp, data)
    return hrp + "1" + "".join(CHARSET[d] for d in combined)


def convertbits(data, from_bits, to_bits, pad=True):
    acc = 0
    bits = 0
    ret = []
    maxv = (1 << to_bits) - 1
    for value in data:
        acc = (acc << from_bits) | value
        bits += from_bits
        while bits >= to_bits:
            bits -= to_bits
            ret.append((acc >> bits) & maxv)
    if pad and bits:
        ret.append((acc << (to_bits - bits)) & maxv)
    return ret


def terp_address(pubkey):
    sha = hashlib.sha256(pubkey).digest()
    ripe = hashlib.new("ripemd160", sha).digest()
    return bech32_encode("terp", convertbits(ripe, 8, 5))


def low_s(s):
    if s > SECP_N // 2:
        s = SECP_N - s
    return s


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--key", required=True)
    parser.add_argument("--method")
    parser.add_argument("--path")
    parser.add_argument("--body")
    parser.add_argument("--domain", default="hash-market.tree-write.v1")
    parser.add_argument("--address-only", action="store_true")
    parser.add_argument("--pubkey-only", action="store_true")
    args = parser.parse_args()
    raw = open(args.key, "rb").read()
    if len(raw) != 32:
        sys.exit("key")
    key = ec.derive_private_key(int.from_bytes(raw, "big"), ec.SECP256K1())
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

    pub = key.public_key().public_bytes(Encoding.X962, PublicFormat.CompressedPoint)
    if args.address_only:
        sys.stdout.write(terp_address(pub))
        return
    if args.pubkey_only:
        sys.stdout.write(pub.hex())
        return
    if not args.method or not args.path or not args.body:
        sys.exit("method path body")
    body = open(args.body, "rb").read()
    body_hash = hashlib.sha256(body).hexdigest()
    challenge = f"{args.domain}\n{args.method.upper()}\n{args.path}\n{body_hash}"
    import base64

    data_b64 = base64.b64encode(challenge.encode()).decode()
    signer = terp_address(pub)
    preamble = (
        '{"account_number":"0","chain_id":"","fee":{"amount":[],"gas":"0"},'
        '"memo":"","msgs":[{"type":"sign/MsgSignData","value":{"data":"'
        + data_b64
        + '","signer":"'
        + signer
        + '"}}],"sequence":"0"}'
    )
    digest = hashlib.sha256(preamble.encode()).digest()
    der = key.sign(digest, ec.ECDSA(Prehashed(hashes.SHA256())))
    r, s = decode_dss_signature(der)
    sig = r.to_bytes(32, "big") + low_s(s).to_bytes(32, "big")
    sys.stdout.write(sig.hex())


if __name__ == "__main__":
    main()
