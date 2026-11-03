#!/usr/bin/env python3
"""Render the Foundation-DAO dry-run flow for the initial headstash (v0.0.1), offline.

Sequence (agreed 2026-11-03):
  1. hs-upload   plain MsgStoreCode of cw-headstash + frontend via cw-orch-kms (mainnet_deploy hs-upload).
                 No x/gov, no DAO: code upload is Everybody on morocco-1.
  2. DAO prop 1  Foundation DAO (proposal module A, via its pre-propose) creates factory/<DAO>/terpy and /thioly,
                 sets metadata, mints exactly the allocation of each to itself, instantiate2s one cw-headstash per
                 token (admin = owner = pauser = DAO), opens the identical campaign on each, funds each, and last
                 renounces the tokenfactory admin of both denoms to NULL_ADMIN (no key), fixing supply and metadata.
                 Submitted by Foundation member SUBMITTER to pre-propose A; it lands in proposal module A.
  3. x/gov       circuit only: phase-b/11-12 (vk_id 3, vk 34d11437), unchanged.
  4. DAO prop 2  bind_note_circuit(zk_id) on both instances (after the circuit is stored).
Nothing is signed or broadcast here. Usage:
  render_dao_flow.py --decisions decisions.mainnet.json --wasm W --fe-wasm F [--code-id N] [--zk-id 1] [--terpd terpd] [--out DIR]
"""
import base64, hashlib, json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
DAO = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w"
PROP_A = "terp1p5582k3sw008jkmw0f48ucszn660nazqr9x3ut6h5aqj4frq8f0shaey4g"
PREPROPOSE_A = "terp1armcr7lme0ac3l59dx6ne3et3pe9mfyv57d8pc9m5tyxtzm64dpq9nnwpp"
GOV = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq"
# Foundation cw4 member (weight 1000) who signs both DAO proposals (group terp1axqxut3zy6n4cpekk9yrwk4u4rny5c4repn7u696g43cufmcszfqq8n2vm).
SUBMITTER = "terp16xmf57fr64n9ajux0up3n6ypytct5zggj5d05r"
# bech32("terp", 20 zero bytes). x/tokenfactory MsgChangeAdmin.ValidateBasic rejects an empty new_admin (AccAddressFromBech32),
# so the renounce target is the all-zero address: valid bech32, no private key, cannot be a contract (32-byte), cannot sign.
NULL_ADMIN = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw"
TOKENS = ("terpy", "thioly")
CTX_TAG = b"terp-hs-direct-ctx-v1"
SUPERSEDED = [
    ("phase-a/13-store-wasm.unsigned.json", "8c38df2a", "eph-key MsgExec store, gov-only instantiate permission: replaced by kms hs-upload (plain store)"),
    ("phase-a/13b-store-frontend-wasm.unsigned.json", "83213af3", "same, frontend"),
    ("phase-a/14-deploy-proposal.unsigned.json", "5e521456 (sign doc) / b14e6bd2 (file)", "x/gov instantiate+fund in uterp: replaced by DAO prop 1 (terpy/thioly)"),
    ("phase-b/14-fund-bind-proposal.unsigned.json", "ec35c009 / 300059c0", "x/gov fund+bind: replaced by DAO prop 1 (fund) + DAO prop 2 (bind)"),
]
KEPT = [("phase-b/11-circuit-proposal.inner.json", "circuit MsgStoreFullCircuit (vk_id 3)"),
        ("phase-b/12-circuit-exec.unsigned.json", "b591de37: x/gov circuit proposal, the only gov step")]

def opt(k, dflt=None):
    a = sys.argv
    return a[a.index(k) + 1] if k in a else dflt

def b64(b): return base64.b64encode(b).decode()
def sha(b): return hashlib.sha256(b).hexdigest()
def derive_nd(raw):
    h = bytearray(subprocess.run(["b3sum", "--no-names"], input=raw.encode(), capture_output=True, check=True).stdout.split()[0].decode().encode())
    h = bytearray.fromhex(h.decode()); h[31] &= 0x1F
    return bytes(h)
def direct_ctx(chain, contract):
    h = bytearray(hashlib.sha256(CTX_TAG + b"\0" + chain.encode() + b"\0" + contract.encode()).digest()); h[31] &= 0x3F
    return bytes(h)

# protobuf (tokenfactory v1beta1, bank Metadata)
def _v(n):
    o = bytearray()
    while True:
        x = n & 0x7F; n >>= 7
        if n: o.append(x | 0x80)
        else: o.append(x); return bytes(o)
def _s(f, s):
    s = s.encode() if isinstance(s, str) else s
    return _v((f << 3) | 2) + _v(len(s)) + s
def _u(f, n): return _v(f << 3) + _v(n)
def stargate(url, raw): return {"stargate": {"type_url": url, "value": b64(raw)}}
def coin(d, a): return _s(1, d) + _s(2, str(a))
def tf_create(sub): return stargate("/osmosis.tokenfactory.v1beta1.MsgCreateDenom", _s(1, DAO) + _s(2, sub))
def tf_mint(denom, amt): return stargate("/osmosis.tokenfactory.v1beta1.MsgMint", _s(1, DAO) + _s(2, coin(denom, amt)) + _s(3, DAO))
def tf_metadata(denom, sub):
    units = _s(2, _s(1, denom) + _u(2, 0)) + _s(2, _s(1, sub) + _u(2, 6))
    md = _s(1, "%s: headstash v0.0.1 initial dry run (Terp Network Foundation)" % sub) + units + _s(3, denom) + _s(4, sub) + _s(5, sub) + _s(6, sub.upper())
    return stargate("/osmosis.tokenfactory.v1beta1.MsgSetDenomMetadata", _s(1, DAO) + _s(2, md))
def tf_change_admin(denom): return stargate("/osmosis.tokenfactory.v1beta1.MsgChangeAdmin", _s(1, DAO) + _s(2, denom) + _s(3, NULL_ADMIN))
def wexec(c, m): return {"wasm": {"execute": {"contract_addr": c, "msg": b64(json.dumps(m, separators=(",", ":")).encode()), "funds": []}}}

def main():
    dec = json.load(open(opt("--decisions", os.path.join(HERE, "decisions.mainnet.json"))))
    terpd = opt("--terpd", "terpd"); out = opt("--out", os.path.join(HERE, "rendered-mainnet", "dao-flow"))
    code_id = opt("--code-id"); zk_id = int(opt("--zk-id", "1"))
    wb = open(opt("--wasm"), "rb").read(); fb = open(opt("--fe-wasm"), "rb").read()
    if sha(wb) != dec["WASM_SHA256"]: sys.exit("FATAL wasm sha %s != %s" % (sha(wb), dec["WASM_SHA256"]))
    if sha(fb) != dec["FE_WASM_SHA256"]: sys.exit("FATAL fe sha %s != %s" % (sha(fb), dec["FE_WASM_SHA256"]))
    base = os.path.join(HERE, "rendered-mainnet", "00-render")
    init0 = json.load(open(os.path.join(base, "01-instantiate.json")))
    camp0 = json.load(open(os.path.join(base, "02-set-epoch-campaign-open.json")))
    c = camp0["set_epoch_campaign"]["campaign"]
    alloc = sum(int(a["amount"]) * int(a["cap"]) for a in c["amounts"]); notes = sum(int(a["cap"]) for a in c["amounts"])
    if str(alloc) != dec["FUNDING_AMOUNT_UTERP"]: sys.exit("FATAL allocation %d != FUNDING_AMOUNT %s" % (alloc, dec["FUNDING_AMOUNT_UTERP"]))
    # self-check: the gov-era render used the same ctx/denom derivation
    gsalt = sha(("headstash:" + dec["CAMPAIGN_LABEL"]).encode())
    gaddr = subprocess.run([terpd, "q", "wasm", "build-address", dec["WASM_SHA256"], GOV, gsalt], capture_output=True, text=True, check=True).stdout.split()[-1]
    assert b64(direct_ctx(dec["CHAIN_ID"], gaddr)) == c["ctx"], "ctx derivation drifted from 00-render"
    assert b64(derive_nd("uterp")) == c["denom"], "derive_nd drifted from 00-render"
    os.makedirs(out, exist_ok=True)
    inst, msgs = {}, []
    for t in TOKENS:
        denom = "factory/%s/%s" % (DAO, t); label = "%s-%s" % (dec["CAMPAIGN_LABEL"], t)
        salt = sha(("headstash:" + label).encode())
        addr = subprocess.run([terpd, "q", "wasm", "build-address", dec["WASM_SHA256"], DAO, salt], capture_output=True, text=True, check=True).stdout.split()[-1]
        nd = derive_nd(denom); ctx = direct_ctx(dec["CHAIN_ID"], addr)
        init = dict(init0, genesis_label=label, token_strategy={"existing_fungible": {"proof": b64(nd), "raw": denom}}, pauser=DAO, frontend_contract=None)
        camp = json.loads(json.dumps(camp0)); cc = camp["set_epoch_campaign"]["campaign"]; cc["ctx"] = b64(ctx); cc["denom"] = b64(nd)
        inst[t] = {"label": label, "denom": denom, "address": addr, "salt_hex": salt, "nd_b64": b64(nd), "ctx_b64": b64(ctx), "allocation": str(alloc)}
        msgs += [tf_create(t), tf_metadata(denom, t), tf_mint(denom, alloc),
                 {"wasm": {"instantiate2": {"admin": DAO, "code_id": int(code_id) if code_id else "{{CODE_ID of cw-headstash from hs-upload}}",
                                            "label": label, "msg": b64(json.dumps(init, separators=(",", ":")).encode()),
                                            "funds": [{"denom": denom, "amount": "1"}], "salt": b64(bytes.fromhex(salt))}}},
                 wexec(addr, camp),
                 {"bank": {"send": {"to_address": addr, "amount": [{"denom": denom, "amount": str(alloc - 1)}]}}}]
        json.dump(init, open(os.path.join(out, "10-%s-instantiate.json" % t), "w"), indent=1)
        json.dump(camp, open(os.path.join(out, "11-%s-set-epoch-campaign.json" % t), "w"), indent=1)
    # renounce last: metadata, mint and funding above all need the DAO as admin
    msgs += [tf_change_admin("factory/%s/%s" % (DAO, t)) for t in TOKENS]
    desc = ("Initial headstash dry run (v0.0.1). Creates two tokenfactory tokens owned by the Foundation DAO, terpy and thioly, "
            "mints exactly the airdrop allocation of each (%s base units, 6 decimals), and deploys one cw-headstash per token "
            "with the identical airdrop set (%d notes, cutoff %s, vk_id 3). Admin, owner and pauser of both: this DAO. "
            "Claims open after x/gov stores the circuit and DAO proposal 2 binds it.") % (alloc, notes, c["cutoff_height"])
    desc += (" Last, the DAO hands the tokenfactory admin of both denoms to %s (the all-zero address, no key), so neither "
             "supply nor metadata can change again.") % NULL_ADMIN
    p1 = {"propose": {"msg": {"propose": {"title": "Headstash v0.0.1 initial dry run: terpy + thioly", "description": desc, "msgs": msgs}}}}
    p2 = {"propose": {"msg": {"propose": {"title": "Headstash v0.0.1: bind the vk_id 3 circuit (zk_id %d)" % zk_id,
          "description": "Binds the x/gov-stored halo2 circuit (vk 34d11437) to both dry-run instances. Check zk_id against the circuit proposal's store event before submitting.",
          "msgs": [wexec(inst[t]["address"], {"bind_note_circuit": {"zk_id": zk_id}}) for t in TOKENS]}}}}
    def unsigned(body, memo):
        return {"body": {"messages": [{"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": SUBMITTER,
                 "contract": PREPROPOSE_A, "msg": body, "funds": []}], "memo": memo, "timeout_height": "0", "extension_options": [], "non_critical_extension_options": []},
                "auth_info": {"signer_infos": [], "fee": {"amount": [{"denom": "uterp", "amount": "{{fee}}"}], "gas_limit": "{{gas}}", "payer": "", "granter": ""}}, "signatures": []}
    files = {
        "20-dao-prop1-deploy.propose.json": p1, "20-dao-prop1-deploy.unsigned.json": unsigned(p1, "headstash v0.0.1 dao prop 1"),
        "40-dao-prop2-bind.propose.json": p2, "40-dao-prop2-bind.unsigned.json": unsigned(p2, "headstash v0.0.1 dao prop 2"),
        "50-vote.template.json": {"vote": {"proposal_id": "{{id}}", "vote": "yes", "rationale": None}},
        "51-execute.template.json": {"execute": {"proposal_id": "{{id}}"}},
        "60-dao-reclaim.template.json": {"emergency_reclaim": {}},
        "61-dao-sweep.template.json": {"sweep_unclaimed": {}},
    }
    for n, v in files.items(): json.dump(v, open(os.path.join(out, n), "w"), indent=1)
    plan = ("# Step 1: plain upload (cw-orch-kms, no gov)\n\n"
            "    cargo run --release --bin mainnet_deploy -- hs-upload --signer kms   # Zigner signs one session grant\n\n"
            "| code | artifact | sha256 | bytes |\n|---|---|---|---|\n"
            "| cw-headstash | artifacts/mainnet/cw_headstash.wasm | %s | %d |\n| cw-headstash-frontend | artifacts/mainnet/cw_headstash_frontend.wasm | %s | %d |\n\n"
            "hs-upload refuses to store a file whose sha256 differs from the pin. kms stores raw wasm with instantiate permission Everybody.\n"
            "After it lands, re-render with `--code-id <cw-headstash code id>` so prop 1 is final.\n"
            "\n# Step 3: x/gov circuit only\n\nUnchanged: phase-b/11-circuit-proposal.inner.json and phase-b/12-circuit-exec.unsigned.json (b591de37; metadata = 02 313fb903).\n") % (
            dec["WASM_SHA256"], len(wb), dec["FE_WASM_SHA256"], len(fb))
    plan += ("\n# Who signs what (DAO steps)\n\n"
             "- Submitter: Foundation cw4 member `%s` (weight 1000 of 3000).\n"
             "- Tx target: pre-propose A `%s`; the proposal lands in **proposal module A** `%s` of DAO `%s`.\n"
             "  Votes (50-vote.template.json) and execute (51-execute.template.json) go to module A, not module B.\n"
             "- Prop 1 ends with MsgChangeAdmin to `%s` for both denoms (after create, metadata, mint, fund).\n") % (SUBMITTER, PREPROPOSE_A, PROP_A, DAO, NULL_ADMIN)
    open(os.path.join(out, "00-upload-plan.md"), "w").write(plan)
    sup = "# Superseded by the DAO flow (2026-11-03)\n\nDo not sign these. They stay in the tree for the record (no deletions).\n\n| file | sha256 prefix | why |\n|---|---|---|\n"
    sup += "".join("| %s | %s | %s |\n" % s for s in SUPERSEDED)
    sup += ("\n## Proposal metadata\n\nproposal-metadata/01-deploy.json and 03-fund-bind.json are no longer submitted (their x/gov "
            "proposals are superseded above). They stay unedited for the record. Only 02-circuit-store.json is used, by phase-b/12.\n")
    sup += "\n## Kept\n\n" + "".join("- %s: %s\n" % k for k in KEPT)
    open(os.path.join(out, "SUPERSEDED.md"), "w").write(sup)
    report = {"dao": DAO, "proposal_module_A": PROP_A, "pre_propose_A": PREPROPOSE_A, "gov": GOV, "submitter": SUBMITTER, "null_admin": NULL_ADMIN, "code_id": code_id, "zk_id": zk_id,
              "allocation_each": str(alloc), "notes": notes, "cutoff_height": c["cutoff_height"], "vk_id": c["vk_id"], "instances": inst,
              "prop1_msgs": len(msgs), "files": {n: sha(open(os.path.join(out, n), "rb").read()) for n in sorted(os.listdir(out)) if n != "render-report.json"}}
    json.dump(report, open(os.path.join(out, "render-report.json"), "w"), indent=1)
    print(json.dumps({k: report[k] for k in ("allocation_each", "notes", "prop1_msgs", "code_id")}), json.dumps({t: inst[t]["address"] for t in TOKENS}))
    for n, h in report["files"].items(): print(h[:16], n)

if __name__ == "__main__":
    main()
