#!/usr/bin/env python3
"""Render the Headstash mainnet deploy messages and gov proposal from templates.

Nothing here signs or broadcasts.

usage:
  render.py DECISIONS.json --circuit DIR [--elig DIR] [--out DIR] [--partial] [--lane]
  render.py --dry-run --circuit DIR [--elig DIR] [--out DIR] [--lane]

Paths
  v1 default is the DIRECT path: the contract verifies note proofs (vk_id 3) and pays at once.
  No hashmerchant, lane, custody key or oracle settings are rendered or required.
  LANE_PATH=true in DECISIONS.json (or --lane) opts in to the optional lane path: adds
  MsgRegisterChain, the custody key as the trusted batch key, 05/08 (final anchor, staged batches),
  and requires SIDECAR_LANE_URL, CUSTODY_PUBKEY_B64 and later FINAL_ANCHOR_B64.

Inputs
  DECISIONS.json  user decisions (see decisions.example.json).
  --circuit DIR   holds params.bin and vk_body.bin of the vk_id 3 note circuit
                  (`bench_note_proofs <dir> 0`). sha256(params||vk_body) must equal
                  NOTE_VK_HASH_HEX below or rendering stops.
  --elig DIR      eligibility build; default ../target/eligibility-denoms. Direct needs
                  notes/tree.json + notes/summary.json (the depth-17 note tree the contract
                  registers against); tree.json/summary.json (identities) give the stats.

Fail-loud rules
  * A user-decision value that is missing, empty, or still a "{{...}}"/DECISION/EXAMPLE
    placeholder stops the render (exit 1). Nothing is written for that run.
  * Later-step values (CONTRACT_ADDR, CODE_ID, FINAL_ANCHOR_B64, NOTE_ZK_ID, ...) are only
    known after an on-chain step. Without --partial any of them missing is exit 1 too; with
    --partial files that need them are skipped and listed, everything else is written.
  * Any "{{" left in an output, or JSON that does not parse, is exit 1.

--dry-run fills every value with obviously fake EXAMPLE data (addresses are valid bech32 of
sha256("EXAMPLE ...") so terpd can parse them) and writes to deploy/rendered-example/. The
real mode refuses any value containing EXAMPLE, so example output cannot leak into a real run.
"""
import base64, collections, copy, hashlib, json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ELIG = os.path.join(HERE, "..", "target", "eligibility-denoms")
TPL, PROP = os.path.join(HERE, "templates"), os.path.join(HERE, "proposals")
PALLAS_P = 0x40000000000000000000000000000000224698fc094cf91b992d30ed00000001
GOV = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq"
# sha256("uterp")-style denom proof used by cw-headstash ExistingFungible (same as the e2e harness).
DENOM_HEX = "1d56a5879c78333d517942a36e7ea0932e7ffa1a1027e439279e8517580ea706"
# vk_id 3 note-membership circuit: sha256(params.bin || vk_body.bin). Campaign vk_hash.
NOTE_VK_HASH_HEX = "34d11437d957554f4008377dece5fac0f047e0e9cc012d15b3d09448a6cc5398"
NOTE_K = 11
NOTE_VK_ID = 3
DIRECT_CTX_TAG = b"terp-hs-direct-ctx-v1"   # contracts/headstash/cw-headstash/src/direct.rs CTX_TAG
# The contract rejects an empty trusted set even though the direct path never checks a batch
# signature. Direct renders put one nothing-up-my-sleeve 32-byte value here (sha256 of a public
# string, no known private key), so no lane batch can ever be signed for this campaign.
DIRECT_TRUSTED_PLACEHOLDER = hashlib.sha256(b"terp-hs-direct-v1: no lane, no trusted batch key").digest()
LANE_ONLY_FILES = {"05-set-epoch-campaign-final-anchor.json", "08-stage-epoch-batch.example.json"}
# Fee-grant estimate (docs: runbook budget): 98,841 notes, 1.3x registration margin.
FEEGRANT_RECOMMENDED_UTHIOL = 4_761_700_000
SOURCES = {
    "community-pool": "community pool (one MsgCommunityPoolSpend of uthiol per operator, in this proposal)",
    "foundation-dao": "foundation DAO (bank send from the DAO to each operator, outside this proposal)",
}

# key -> kind. decision = user must choose; later = known after an on-chain step.
DECISION = {
    "CHAIN_ID": "str", "CAMPAIGN_LABEL": "str", "CONTRACT_OWNER": "addr", "WASM_ADMIN": "addr",
    "WAVS_POO_JSON": "json",
    "CUTOFF_HEIGHT": "int", "ANCHOR_START_HEIGHT": "int", "ANCHOR_INTERVAL": "int",
    "MIN_ANONYMITY_SET": "int", "MAX_ROWS": "int", "SWEEP_RECIPIENT": "addr",
    "FUNDING_AMOUNT_UTERP": "int", "DEPOSIT_UTERP": "int", "PROPOSER": "addr",
    "SOURCE_COMMIT": "hex", "FEEGRANT_FUNDING_SOURCE": "source", "OPERATORS": "operators",
}
LATER = {
    "CONTRACT_ADDR": "addr", "CODE_ID": "int", "WASM_SHA256": "hex", "PROPOSAL_METADATA_URI": "url",
    "FORUM_URL": "url", "NOTE_ZK_ID": "int",
}
LANE_DECISION = {"CUSTODY_PUBKEY_B64": "b64_32", "SIDECAR_LANE_URL": "url"}
LANE_LATER = {"FINAL_ANCHOR_B64": "b64_32"}

# ---- bech32 (BIP-173) ----
CS = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
def _pm(v):
    g = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]; c = 1
    for x in v:
        b = c >> 25; c = (c & 0x1ffffff) << 5 ^ x
        for i in range(5): c ^= g[i] if (b >> i) & 1 else 0
    return c
def _hx(h): return [ord(x) >> 5 for x in h] + [0] + [ord(x) & 31 for x in h]
def _cv(data, f, t, pad):
    acc = bits = 0; out = []
    for x in data:
        acc = (acc << f) | x; bits += f
        while bits >= t: bits -= t; out.append((acc >> bits) & ((1 << t) - 1))
    if pad and bits: out.append((acc << (t - bits)) & ((1 << t) - 1))
    elif not pad and (bits >= f or (acc << (t - bits)) & ((1 << t) - 1)): return None
    return out
def bech32_encode(hrp, raw):
    d = _cv(raw, 8, 5, True); p = _pm(_hx(hrp) + d + [0] * 6) ^ 1
    return hrp + "1" + "".join(CS[x] for x in d + [(p >> 5 * (5 - i)) & 31 for i in range(6)])
def is_terp_addr(s):
    if not isinstance(s, str) or s.lower() != s or not s.startswith("terp1"): return False
    d = [CS.find(c) for c in s[5:]]
    if -1 in d or len(d) < 7 or _pm(_hx("terp") + d) != 1: return False
    raw = _cv(d[:-6], 5, 8, False)
    return raw is not None and len(raw) in (20, 32)

def b64(b): return base64.b64encode(b).decode()
def thiol(u): s = f"{u / 1e6:,.6f}".rstrip("0").rstrip("."); return s
def ex_addr(tag, n=20): return bech32_encode("terp", hashlib.sha256(b"EXAMPLE " + tag.encode()).digest()[:n])

def direct_ctx(chain_id, contract):
    """Mirror of cw-headstash direct::direct_ctx: sha256(tag||0||chain_id||0||contract), top 2 bits cleared."""
    h = bytearray(hashlib.sha256(DIRECT_CTX_TAG + b"\0" + chain_id.encode() + b"\0" + contract.encode()).digest())
    h[31] &= 0x3F
    assert int.from_bytes(h, "little") < PALLAS_P
    return bytes(h)

def derive_ctx(chain_id, contract, label):
    """Lane path only. ctx = sha256(tag||chain_id||0||contract||0||label), top 2 bits cleared (LE repr < p)."""
    h = bytearray(hashlib.sha256(b"terp-hs-ctx-v1" + chain_id.encode() + b"\0" + contract.encode() + b"\0" + label.encode()).digest())
    h[31] &= 0x3F
    assert int.from_bytes(h, "little") < PALLAS_P
    return bytes(h)

def example_decisions():
    return {
        "CHAIN_ID": "morocco-1", "CAMPAIGN_LABEL": "headstash-EXAMPLE",
        "CONTRACT_OWNER": ex_addr("owner"), "WASM_ADMIN": GOV,
        "WAVS_POO_JSON": {"EXAMPLE": "operator set PoP"},
        "CUTOFF_HEIGHT": 99999999, "ANCHOR_START_HEIGHT": 11111111, "ANCHOR_INTERVAL": 14400,
        "MIN_ANONYMITY_SET": 64, "MAX_ROWS": 25, "SWEEP_RECIPIENT": GOV,
        "FUNDING_AMOUNT_UTERP": 1234567, "DEPOSIT_UTERP": 1000000, "PROPOSER": ex_addr("proposer"),
        "SOURCE_COMMIT": "0" * 40, "FEEGRANT_FUNDING_SOURCE": "community-pool",
        "OPERATORS": [
            {"label": "EXAMPLE operator A", "addr": ex_addr("operator A"), "budget_uthiol": 2_400_000_000},
            {"label": "EXAMPLE operator B", "addr": ex_addr("operator B"), "budget_uthiol": 1_600_000_000},
            {"label": "EXAMPLE operator C", "addr": ex_addr("operator C"), "budget_uthiol": 800_000_000},
        ],
        "CONTRACT_ADDR": ex_addr("contract", 32), "CODE_ID": 999999,
        "WASM_SHA256": "3010d1baf4623a665f0631cb099af973f3de97b2fdb62a5e798f582d4e0a597b",
        "PROPOSAL_METADATA_URI": "https://example.invalid/EXAMPLE-proposal.json",
        "FORUM_URL": "https://example.invalid/EXAMPLE-forum",
        "NOTE_ZK_ID": 999,
    }

def example_lane():
    return {"CUSTODY_PUBKEY_B64": b64(hashlib.sha256(b"EXAMPLE custody").digest()),
            "SIDECAR_LANE_URL": "https://example.invalid/EXAMPLE-lane",
            "FINAL_ANCHOR_B64": b64(hashlib.sha256(b"EXAMPLE anchor").digest())}

def unfilled(x):
    return x is None or (isinstance(x, str) and (x.strip() == "" or "{{" in x or "DECISION" in x.upper()))

def check(key, kind, val, dry):
    """Return an error string or None."""
    if unfilled(val): return "unfilled"
    if not dry and "EXAMPLE" in json.dumps(val).upper(): return "EXAMPLE value in a real render"
    s = str(val)
    if kind == "int" and not re.fullmatch(r"[1-9][0-9]*", s): return f"not a positive integer: {s!r}"
    if kind == "addr" and not is_terp_addr(s): return f"not a terp1 bech32 address: {s!r}"
    if kind == "hex" and not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", s): return f"not lowercase hex sha: {s!r}"
    if kind == "url" and not re.fullmatch(r"https://\S+", s): return f"not an https URL: {s!r}"
    if kind == "b64_32":
        try:
            if len(base64.b64decode(s, validate=True)) != 32: return "not 32 bytes"
        except Exception: return "not base64"
    if kind == "source" and s not in SOURCES: return f"must be one of {sorted(SOURCES)}"
    if kind == "json" and not isinstance(val, (dict, list)): return "must be a JSON object/array (not a string)"
    if kind == "operators":
        if not isinstance(val, list) or not val: return "must be a non-empty list of {label, addr, budget_uthiol}"
        seen = set()
        for i, o in enumerate(val, 1):
            if not isinstance(o, dict): return f"operator {i} is not an object"
            for f in ("label", "addr", "budget_uthiol"):
                e = check(f"OPERATORS[{i}].{f}", {"addr": "addr", "budget_uthiol": "int"}.get(f, "str"), o.get(f), dry)
                if e: return f"operator {i} {f}: {e}"
            if o["addr"] in seen: return f"operator {i} duplicate addr"
            seen.add(o["addr"])
    return None

def load_circuit(d):
    pb = open(os.path.join(d, "params.bin"), "rb").read()
    vb = open(os.path.join(d, "vk_body.bin"), "rb").read()
    h = hashlib.sha256(pb + vb).hexdigest()
    if h != NOTE_VK_HASH_HEX:
        sys.exit(f"FATAL circuit {d}: sha256(params||vk_body) = {h}, expected {NOTE_VK_HASH_HEX}")
    if int.from_bytes(pb[:4], "little") != NOTE_K:
        sys.exit(f"FATAL circuit {d}: params k = {int.from_bytes(pb[:4], 'little')}, expected {NOTE_K}")
    return pb, vb, h

def build_proposal(text, v, ops, source, lane):
    """Expand operator prototype messages (dropped for the foundation-DAO source); drop the
    hashmerchant MsgRegisterChain unless the lane path is opted in."""
    p = json.loads(text)
    msgs, proto = [], None
    for m in p["messages"]:
        if m["@type"] == "/terp.hashmerchant.v1.MsgRegisterChain" and not lane: continue
        if re.fullmatch(r"\{\{OPERATOR_\d+_ADDR\}\}", str(m.get("recipient", ""))):
            proto = proto or m; continue
        msgs.append(m)
    if proto is None: sys.exit("FATAL submit-proposal template has no operator prototype message")
    if source == "community-pool":
        for i, _ in enumerate(ops, 1):
            m = copy.deepcopy(proto)
            m["recipient"] = f"{{{{OPERATOR_{i}_ADDR}}}}"
            m["amount"][0]["amount"] = f"{{{{OPERATOR_{i}_BUDGET_UTHIOL}}}}"
            msgs.append(m)
    p["messages"] = msgs
    if source != "community-pool":
        p["summary"] = p["summary"].replace(" Sends each named sidecar operator its THIOL fee-grant budget.",
                                            " Operator THIOL fee-grant budgets come from the foundation DAO, not this proposal.")
    return json.dumps(p, indent=2) + "\n"

def cond_blocks(text, lane):
    """Keep <!--LANE-->..<!--/LANE--> only on the lane path and <!--DIRECT-->..<!--/DIRECT--> only on direct."""
    keep, drop = ("LANE", "DIRECT") if lane else ("DIRECT", "LANE")
    text = re.sub(r"<!--%s-->\n?(.*?)<!--/%s-->\n?" % (drop, drop), "", text, flags=re.S)
    return re.sub(r"<!--/?%s-->\n?" % keep, "", text)

def as_bool(k, x):
    if x is None: return False
    if isinstance(x, bool): return x
    if isinstance(x, str) and x.lower() in ("true", "false"): return x.lower() == "true"
    sys.exit(f"FATAL {k} must be true or false, got {x!r}")

def main():
    a = sys.argv[1:]
    if not a or "-h" in a or "--help" in a: sys.exit(__doc__)
    opt = lambda k, d=None: a[a.index(k) + 1] if k in a else d
    dry, partial = "--dry-run" in a, "--partial" in a
    circuit = opt("--circuit")
    if not circuit: sys.exit("FATAL --circuit DIR is required (params.bin + vk_body.bin of the vk_id 3 circuit)")
    elig = opt("--elig", ELIG)
    if dry:
        dec = example_decisions()
    else:
        pos = [x for i, x in enumerate(a) if not x.startswith("--") and (i == 0 or a[i - 1] not in ("--circuit", "--elig", "--out"))]
        if not pos: sys.exit("FATAL DECISIONS.json is required (or --dry-run)")
        dec = json.load(open(pos[0]))
    lane = as_bool("LANE_PATH", dec.get("LANE_PATH")) or "--lane" in a
    if dry and lane: dec.update(example_lane())
    out = opt("--out", os.path.join(HERE, ("rendered-example" if dry else "rendered") + ("-lane" if lane else "")))
    need_dec = dict(DECISION, **(LANE_DECISION if lane else {}))
    need_later = dict(LATER, **(LANE_LATER if lane else {}))
    errors, later_missing = [], set()
    for k, kind in need_dec.items():
        e = check(k, kind, dec.get(k), dry)
        if e: errors.append(f"  {k}: {e}")
    for k, kind in need_later.items():
        e = check(k, kind, dec.get(k), dry)
        if e == "unfilled": later_missing.add(k)
        elif e: errors.append(f"  {k}: {e}")
    if not lane:
        stray = sorted(k for k in list(LANE_DECISION) + list(LANE_LATER) if not unfilled(dec.get(k)))
        if stray: print("NOTE LANE_PATH is false: ignoring lane-only values " + ", ".join(stray), file=sys.stderr)
        for k in list(LANE_DECISION) + list(LANE_LATER): dec.pop(k, None)
    if errors:
        print("FATAL user decisions not filled or invalid (nothing written):", *errors, sep="\n", file=sys.stderr); sys.exit(1)
    if later_missing and not partial:
        print("FATAL later-step values missing (rerun with --partial to render the files that do not need them):",
              *[f"  {k}" for k in sorted(later_missing)], sep="\n", file=sys.stderr); sys.exit(1)

    pb, vb, vkh = load_circuit(circuit)
    tree = json.load(open(os.path.join(elig, "tree.json")))
    summ = json.load(open(os.path.join(elig, "summary.json")))
    id_amounts = collections.Counter(int(e["amount"]) for e in tree["entries"])
    total = sum(int(e["amount"]) for e in tree["entries"])
    if lane:   # lane: campaign registers against the identity tree, one amount per identity
        root_hex, depth, classes = tree["root_hex"], tree["depth"], sorted(id_amounts.items())
        notes_total, nsumm = None, None
    else:      # direct: the depth-17 fixed-denomination note tree (contract checks DENOMS_UTERP)
        nd = os.path.join(elig, "notes")
        if not os.path.exists(os.path.join(nd, "tree.json")):
            sys.exit(f"FATAL direct path needs the note build {nd}/tree.json (eligibility --denominations)")
        ntree, nsumm = json.load(open(os.path.join(nd, "tree.json"))), json.load(open(os.path.join(nd, "summary.json")))
        if ntree["root_hex"] != nsumm["root_hex"] or not nsumm.get("invariant_ok"):
            sys.exit("FATAL note tree.json and summary.json disagree or invariant_ok is false")
        root_hex, depth = ntree["root_hex"], ntree["depth"]
        classes = [(c["amount_uterp"], c["notes"]) for c in nsumm["classes"]]
        notes_total = sum(x * c for x, c in classes)
        assert notes_total == int(nsumm["notes_uterp"]) and sum(c for _, c in classes) == nsumm["notes"]
        if dry: dec["FUNDING_AMOUNT_UTERP"] = str(notes_total)
        if int(dec["FUNDING_AMOUNT_UTERP"]) < notes_total:
            sys.exit(f"FATAL FUNDING_AMOUNT_UTERP {dec['FUNDING_AMOUNT_UTERP']} < note total {notes_total}: "
                     "the contract could not pay every note")
    ops, source = dec["OPERATORS"], dec["FEEGRANT_FUNDING_SOURCE"]
    budget = sum(int(o["budget_uthiol"]) for o in ops)

    v = {k: (json.dumps(x) if isinstance(x, (dict, list)) else str(x)) for k, x in dec.items()
         if k not in ("OPERATORS", "LANE_PATH") and not unfilled(x)}
    v.update({
        "ELIGIBILITY_ROOT": root_hex, "ELIGIBILITY_ROOT_B64": b64(bytes.fromhex(root_hex)),
        "ELIGIBILITY_DEPTH": str(depth), "ELIGIBILITY_IDENTITIES": str(len(tree["entries"])),
        "ELIGIBILITY_TOTAL_UTERP": str(total), "DENOM_B64": b64(bytes.fromhex(DENOM_HEX)), "DENOM_HEX": DENOM_HEX,
        "AMOUNT_CLASSES_JSON": json.dumps([{"amount": x, "cap": c} for x, c in classes]),
        "NOTE_CIRCUIT_PARAMS_B64": b64(pb), "NOTE_CIRCUIT_VK_B64": b64(vb), "NOTE_VK_HASH": vkh,
        "NOTE_VK_HASH_B64": b64(bytes.fromhex(vkh)), "NOTE_VK_ID": str(NOTE_VK_ID), "NOTE_K": str(NOTE_K),
        "FEEGRANT_BUDGET_UTHIOL": str(budget), "FEEGRANT_BUDGET_THIOL": thiol(budget),
        "FEEGRANT_FUNDING_SOURCE": SOURCES[source],
        "TRUSTED_KEYS_JSON": json.dumps([v_ for v_ in [dec.get("CUSTODY_PUBKEY_B64")]] if lane else [b64(DIRECT_TRUSTED_PLACEHOLDER)]),
        "PATH_SUMMARY": ("It also registers the optional headstash-claim oracle source with hashmerchant (lane path)."
                         if lane else "Claims are verified and paid by the contract directly; no hashmerchant registration."),
    })
    if nsumm:
        v.update({"NOTES_COUNT": str(nsumm["notes"]), "NOTES_TOTAL_UTERP": str(notes_total),
                  "NOTES_REMAINDER_UTERP": str(nsumm["remainder_uterp"]), "NOTE_CLASSES": str(len(classes)),
                  "NOTE_MIN_CLASS_NOTES": str(min(c for _, c in classes)),
                  "NOTE_MIN_CLASS_HOLDERS": str(min(c["holders"] for c in nsumm["classes"]))})
    rows = ["| # | Operator | Address | Budget (THIOL) | uthiol |", "|---|---|---|---:|---:|"]
    for i, o in enumerate(ops, 1):
        v[f"OPERATOR_{i}_ADDR"], v[f"OPERATOR_{i}_BUDGET_UTHIOL"] = o["addr"], str(int(o["budget_uthiol"]))
        v[f"OPERATOR_{i}_LABEL"] = o["label"]
        rows.append(f"| {i} | {o['label']} | `{o['addr']}` | {thiol(int(o['budget_uthiol']))} | {int(o['budget_uthiol'])} |")
    rows.append(f"| | **Total** | | **{thiol(budget)}** | **{budget}** |")
    v["OPERATOR_BUDGET_TABLE"] = "\n".join(rows)
    if "CONTRACT_ADDR" in v:
        ctx = derive_ctx(v["CHAIN_ID"], v["CONTRACT_ADDR"], v["CAMPAIGN_LABEL"]) if lane else direct_ctx(v["CHAIN_ID"], v["CONTRACT_ADDR"])
        v["CTX_HEX"], v["CTX_B64"] = ctx.hex(), b64(ctx)

    srcs = [(TPL, n, n) for n in sorted(os.listdir(TPL)) if lane or n not in LANE_ONLY_FILES]
    srcs += [(PROP, n, n) for n in sorted(os.listdir(PROP)) if n.endswith(".md") and n != "README.md"]
    srcs += [(PROP, "submit-proposal.template.json", "submit-proposal.json")]
    rendered, skipped, unknown = {}, {}, set()
    for d, name, oname in srcs:
        text = cond_blocks(open(os.path.join(d, name)).read(), lane)
        if oname == "submit-proposal.json": text = build_proposal(text, v, ops, source, lane)
        need = set()
        def sub(m):
            k = m.group(1)
            if k in v: return v[k]
            (need if k in need_later or k in ("CTX_HEX", "CTX_B64") else unknown).add(k); return m.group(0)
        text = re.sub(r"\{\{([A-Z0-9_]+)\}\}", sub, text)
        if unknown: continue
        if need: skipped[oname] = sorted(need); continue
        if "{{" in text or "<!--LANE" in text or "<!--DIRECT" in text: sys.exit(f"FATAL {oname}: placeholder or block marker left")
        if not lane:
            for bad in ("hashmerchant.v1", "MsgRegisterChain", "sidecar_lane", "custody"):
                if bad in text and oname.endswith(".json"): sys.exit(f"FATAL direct render {oname} still contains {bad}")
        if oname.endswith(".json"): json.loads(text)
        rendered[oname] = text
    if unknown:
        sys.exit("FATAL templates use placeholders render.py does not know: " + ", ".join(sorted(unknown)))

    os.makedirs(out, exist_ok=True)
    for f in os.listdir(out):
        if f.endswith((".json", ".md")): os.remove(os.path.join(out, f))
    for oname, text in rendered.items():
        open(os.path.join(out, oname), "w").write(text)
    prop = json.loads(rendered["submit-proposal.json"]) if "submit-proposal.json" in rendered else None
    rep = {
        "mode": "DRY RUN - EXAMPLE VALUES, DO NOT SUBMIT" if dry else "real",
        "path": "lane (opt-in)" if lane else "direct",
        "campaign_eligibility_root": root_hex, "campaign_eligibility_depth": depth,
        "amount_classes": len(classes), "identities": len(tree["entries"]), "rows": summ.get("rows"),
        "identity_leaf_total_uterp": total, "unallocated_vs_67M_uterp": 67_000_000_000_000 - total,
        "notes": nsumm["notes"] if nsumm else None, "notes_total_uterp": notes_total,
        "notes_remainder_uterp": int(nsumm["remainder_uterp"]) if nsumm else None,
        "unique_amount_identities": None if not lane else sum(1 for c in id_amounts.values() if c == 1),
        "ctx_hex": v.get("CTX_HEX"), "ctx_derivation": "lane derive_ctx (with label)" if lane else "direct_ctx (terp-hs-direct-ctx-v1)",
        "trusted": json.loads(v["TRUSTED_KEYS_JSON"]),
        "note_vk_hash": vkh, "note_params_bytes": len(pb), "note_vk_bytes": len(vb),
        "contract_owner": v["CONTRACT_OWNER"], "funding_amount_uterp": v["FUNDING_AMOUNT_UTERP"],
        "feegrant_source": source, "feegrant_budget_uthiol": budget, "operators": len(ops),
        "feegrant_below_recommended": budget < FEEGRANT_RECOMMENDED_UTHIOL,
        "proposal_messages": [m["@type"] for m in prop["messages"]] if prop else None,
        "required_decisions": sorted(need_dec), "required_later": sorted(need_later),
        "written": sorted(rendered), "skipped_waiting_on": skipped,
        "lane_only_files_omitted": [] if lane else sorted(LANE_ONLY_FILES),
    }
    json.dump(rep, open(os.path.join(out, "render-report.json"), "w"), indent=2)
    print(json.dumps(rep, indent=2))
    if rep["feegrant_below_recommended"]:
        print(f"WARNING fee-grant budget {thiol(budget)} THIOL < recommended {thiol(FEEGRANT_RECOMMENDED_UTHIOL)} THIOL", file=sys.stderr)
    if skipped: print("PARTIAL: some files wait on later-step values (see skipped_waiting_on)", file=sys.stderr)


if __name__ == "__main__":
    main()
