#!/usr/bin/env python3
"""Render the Headstash mainnet deploy messages from templates.

Reads the eligibility build in tools/headstash/target/eligibility/ (tree.json,
summary.json) and a decisions file, fills {{PLACEHOLDERS}} in deploy/templates/,
and writes deploy/rendered/. Nothing here signs or broadcasts.

usage: render.py DECISIONS.json [--elig DIR]
Unfilled {{X}} placeholders are listed and the script exits 1.
"""
import base64, collections, hashlib, json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ELIG = os.path.join(HERE, "..", "target", "eligibility")
TPL, OUT = os.path.join(HERE, "templates"), os.path.join(HERE, "rendered")
PALLAS_P = 0x40000000000000000000000000000000224698fc094cf91b992d30ed00000001
# sha256("uterp")-style denom proof used by cw-headstash ExistingFungible (same as the e2e harness).
DENOM_HEX = "1d56a5879c78333d517942a36e7ea0932e7ffa1a1027e439279e8517580ea706"
# SHA-256 of the membership circuit blob (vk id 2), from membership_vk_hash.
VK_HASH_HEX = "94d4326108e28451829b68d64df58cf975fafdb874a5a554f3cbe8931eefed11"


def b64(b): return base64.b64encode(b).decode()


def derive_ctx(chain_id, contract, label):
    """ctx = sha256(tag||chain_id||0||contract||0||label), top 2 bits cleared (LE repr < p)."""
    h = bytearray(hashlib.sha256(b"terp-hs-ctx-v1" + chain_id.encode() + b"\0" + contract.encode() + b"\0" + label.encode()).digest())
    h[31] &= 0x3F
    assert int.from_bytes(h, "little") < PALLAS_P
    return bytes(h)


def main():
    args = sys.argv[1:]
    if not args: sys.exit(__doc__)
    dec = json.load(open(args[0]))
    elig = args[args.index("--elig") + 1] if "--elig" in args else ELIG
    tree = json.load(open(os.path.join(elig, "tree.json")))
    summ = json.load(open(os.path.join(elig, "summary.json")))
    amounts = collections.Counter(int(e["amount"]) for e in tree["entries"])
    total = sum(int(e["amount"]) for e in tree["entries"])
    v = dict(dec)
    v["ELIGIBILITY_ROOT"] = tree["root_hex"]
    v["ELIGIBILITY_ROOT_B64"] = b64(bytes.fromhex(tree["root_hex"]))
    v["ELIGIBILITY_DEPTH"] = str(tree["depth"])
    v["ELIGIBILITY_IDENTITIES"] = str(len(tree["entries"]))
    v["ELIGIBILITY_TOTAL_UTERP"] = str(total)
    v["DENOM_B64"] = b64(bytes.fromhex(DENOM_HEX))
    v["DENOM_HEX"] = DENOM_HEX
    v["VK_HASH_B64"] = b64(bytes.fromhex(VK_HASH_HEX))
    v["AMOUNT_CLASSES_JSON"] = json.dumps([{"amount": a, "cap": c} for a, c in sorted(amounts.items())])
    v["MAX_ROWS"] = str(dec.get("MAX_ROWS", 25))
    if dec.get("CONTRACT_ADDR") and not dec["CONTRACT_ADDR"].startswith("{{"):
        ctx = derive_ctx(dec["CHAIN_ID"], dec["CONTRACT_ADDR"], dec["CAMPAIGN_LABEL"])
        v["CTX_HEX"], v["CTX_B64"] = ctx.hex(), b64(ctx)
    os.makedirs(OUT, exist_ok=True)
    missing = set()
    PROP = os.path.join(HERE, "proposals")
    srcs = [(TPL, n) for n in sorted(os.listdir(TPL))] + [(PROP, n) for n in sorted(os.listdir(PROP)) if n.endswith(".md") and n != "README.md"]
    for d, name in srcs:
        text = open(os.path.join(d, name)).read()
        def sub(m):
            k = m.group(1)
            if k in v and not str(v[k]).startswith("{{"): return str(v[k])
            missing.add(k); return m.group(0)
        text = re.sub(r"\{\{([A-Z0-9_]+)\}\}", sub, text)
        open(os.path.join(OUT, name), "w").write(text)
        if name.endswith(".json") and not re.search(r"\{\{", text): json.loads(text)
    rep = {"eligibility_root": tree["root_hex"], "identities": len(tree["entries"]), "rows": summ.get("rows"),
           "total_leaf_uterp": total, "unallocated_vs_67M_uterp": 67_000_000_000_000 - total,
           "amount_classes": len(amounts), "unique_amount_identities": sum(1 for c in amounts.values() if c == 1),
           "ctx_hex": v.get("CTX_HEX")}
    json.dump(rep, open(os.path.join(OUT, "render-report.json"), "w"), indent=2)
    print(json.dumps(rep, indent=2))
    if missing:
        print("UNFILLED:", ", ".join(sorted(missing))); sys.exit(1)


if __name__ == "__main__":
    main()
