#!/usr/bin/env python3
"""Headstash v1 direct on-chain claim e2e (vk_id 3 notes, loopback only).

Store the note circuit (store-full-circuit), instantiate cw-headstash (zk-api build),
BindNoteCircuit, chunked RegisterNotes from fee-granted fresh wallets, accept anchors,
then ClaimNote from one fresh fee-granted wallet per note, with exact balance checks
and the negatives from the launch brief. Fee grants come from headstash-register-relay
on 127.0.0.1:18795 (granter = keyring name "relayer"). Test secrets stay in memory.
"""
import base64, hashlib, json, os, secrets, subprocess, sys, time
import requests
import run as b
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "deploy"))
import gov_session as gs
from run import (R, log, step, save, b64, h2b64, pmul, kec, eip191, evm_sign, b58, pallas_rand, terpd, q, height,
                 wait_tx, tx, attr, smart, bal, rec, list_hash, gas_used, N, HOME, CHAIN_ID, NODE, RPC, REST, RUN,
                 RELAY_BIND, RELAY_BIN, ELIG_BIN, EX, WAVS_BIN, DENOM, NOTE_VK_HASH, DENOMS, NOTE_EX, CHUNK, TERPD)
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization


def all_anchors(contract):
    out, after = [], None
    while True:
        page = smart(contract, {"registration_anchors": {"start_after": after, "limit": 100}})["anchors"]
        out += page
        if len(page) < 100: return out
        after = page[-1]["seq"]

WASM = os.environ.get("WASM", "/tmp/hsdirect/opt-out/cw_headstash.wasm")
CUT_AHEAD = int(os.environ.get("CUT_AHEAD", "600"))
SWEEP = os.environ.get("SWEEP", "1") == "1"
# Registration hook: e2e-only recorder contract (tools/headstash/e2e/hook-recorder). HEADSTASH_HOOK=0 = no hook (gas baseline).
HOOK = os.environ.get("HEADSTASH_HOOK", "1") == "1"
HOOK_WASM = os.environ.get("HOOK_WASM", "/tmp/hsdirect/opt-out/hook_recorder.wasm")
REWARD = {}
RELAY_URL = "http://" + RELAY_BIND
CLAIM_GAS = int(os.environ.get("HEADSTASH_CLAIM_GAS", "300000"))
REGISTER_GAS = int(os.environ.get("HEADSTASH_REGISTER_GAS", "12000000"))
GP = 0.025
CTX_TAG = b"terp-hs-direct-ctx-v1"

def direct_ctx(chain_id, contract):
    h = bytearray(hashlib.sha256(CTX_TAG + b"\0" + chain_id.encode() + b"\0" + contract.encode()).digest()); h[31] &= 0x3F
    return bytes(h)

def txg(frm, gas, fee, granter, *args):
    """Fee-granted tx: fee == the allowance spend limit, so one use exhausts it."""
    a = ["tx", *args, "--from", frm, "--home", HOME, "--keyring-backend", "test", "--chain-id", CHAIN_ID, "--node", NODE,
         "--gas", str(gas), "--fees", "%duterp" % fee, "-b", "sync", "-y", "-o", "json"]
    if granter: a += ["--fee-granter", granter]
    p = subprocess.run([TERPD, *a], capture_output=True, text=True)
    try: res = json.loads(p.stdout)
    except Exception: return {"code": -1, "raw_log": (p.stdout + p.stderr)[-600:]}
    if res.get("code", 0) != 0: return res
    return wait_tx(res["txhash"])

def neg(name, res, *needles):
    log_ = res.get("raw_log", "") if isinstance(res, dict) else str(res)
    R["negatives"][name] = "code %s %s" % (res.get("code") if isinstance(res, dict) else "-", log_[:260])
    ok = (res.get("code") != 0 if isinstance(res, dict) else True) and all(n in log_ for n in needles)
    step("neg_" + name, ok, R["negatives"][name])

def grant(grantee, kind, body):
    r = requests.post(RELAY_URL + "/headstash/feegrant", json={"grantee": grantee, kind: body}, timeout=120)
    try: j = r.json()
    except Exception: j = {"error": r.text[:200]}
    return r.status_code, j

def new_wallet(name):
    terpd("keys", "delete", name, "-y", "--keyring-backend", "test", "--home", HOME, check=False)  # stale throwaway key from a prior run
    terpd("keys", "add", name, "--keyring-backend", "test", "--home", HOME, "--output", "json")  # mnemonic discarded
    return terpd("keys", "show", name, "-a", "--keyring-backend", "test", "--home", HOME).strip()

def supply(h):
    r = requests.get(REST + "/cosmos/bank/v1beta1/supply/by_denom", params={"denom": "uterp"},
                     headers={"x-cosmos-block-height": str(h)}, timeout=10).json()
    return int(r["amount"]["amount"])

def minted_at(h):
    """uterp minted by x/mint in block h (inflation lands in the fee collector, not burned)."""
    r = requests.get(RPC + "/block_results", params={"height": h}, timeout=10).json()["result"]
    m = 0
    for e in (r.get("finalize_block_events") or []) + (r.get("begin_block_events") or []):
        if e.get("type") == "mint":
            for a in e.get("attributes", []):
                if a.get("key") == "amount" and a.get("value", "").isdigit(): m += int(a["value"])
    return m

GOV_ROLES = os.environ.get("GOV_ROLES") == "1"
GOV = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq"
INIT_DEPOSIT = 5000000   # local analogue of mainnet 5,000 TERP initial
FULL_DEPOSIT = 10000000  # local analogue of mainnet 10,000 TERP min deposit
SESSION_FEE = 2000000          # eph MsgExec fee, paid by the proposer feegrant
SESSION_ALLOWANCE = 300000000  # feegrant cap sized for a ~175 KB MsgExec (mainnet analogue: 300 THIOL)
# Optional gov frontend hook (set by gov_frontend_e2e.py): FE_GOV = {"wasm": path, "manifest": callable -> manifest dict,
# "max_chunk_bytes": int}. With it, the eph key also stores the frontend wasm, and the contract proposal instantiates the
# frontend (gov admin + owner), publishes the manifest (activate) and sets frontend_contract in the cw-headstash instantiate.
FE_GOV = None
FE_SALT = hashlib.sha256(b"headstash-frontend-gov-e2e").hexdigest()

def eph_store(name, path, PROPOSER, gas):
    """Rendered-mainnet shape (deploy/render_gov_session.py 13/13b): normal MsgStoreCode signed by the eph key,
    sender = eph, instantiate permission gov only, fee paid by the session feegrant (granter = proposer)."""
    import gzip as _gz
    eph = R["addresses"]["EPH_ADDR"]
    m = {"@type": "/cosmwasm.wasm.v1.MsgStoreCode", "sender": eph,
         "wasm_byte_code": base64.b64encode(_gz.compress(open(path, "rb").read(), 9, mtime=0)).decode(),
         "instantiate_permission": {"permission": "AnyOfAddresses", "addresses": [GOV]}}
    res = sign_send("eph", gs._tx([m], gas, int(gas * 0.025) + 1, "uterp", PROPOSER, "headstash-gov-session:store-code"), "store_" + name)
    cid = attr(res, "code_id")
    ci = q("wasm", "code-info", cid) if cid else {}
    perm = ci.get("instantiate_permission", {})
    step("eph_store_%s" % name, res.get("code") == 0 and bool(cid) and perm.get("addresses") == [GOV],
         "code_id %s sha %s gas %d perm %s %s" % (cid, (ci.get("checksum") or ci.get("data_hash") or "").lower(), gas_used(res), json.dumps(perm), res.get("raw_log", "")[:160]))
    R.setdefault("gas", {})["eph_store_" + name] = gas_used(res)
    return cid, (ci.get("checksum") or ci.get("data_hash") or "").lower()

def gen_only(frm, *args):
    p = subprocess.run([TERPD, "tx", *args, "--from", frm, "--home", HOME, "--keyring-backend", "test", "--chain-id", CHAIN_ID,
                        "--node", NODE, "--generate-only", "-o", "json"], capture_output=True, text=True)
    if p.returncode: raise RuntimeError("generate-only %s: %s" % (args[:3], p.stderr[-300:]))
    return p.stdout

def sign_send(frm, unsigned, name, mode=None):
    """Sign an unsigned tx JSON (from gov_session) with a local test key and broadcast to 127.0.0.1 only."""
    uf = os.path.join(RUN, "unsigned_%s.json" % name); sf = os.path.join(RUN, "signed_%s.json" % name)
    json.dump(unsigned, open(uf, "w"))
    a = [TERPD, "tx", "sign", uf, "--from", frm, "--home", HOME, "--keyring-backend", "test", "--chain-id", CHAIN_ID, "--node", NODE, "-o", "json"]
    if mode: a += ["--sign-mode", mode]
    p = subprocess.run(a, capture_output=True, text=True)
    if p.returncode: return {"code": -1, "raw_log": p.stderr[-400:]}
    open(sf, "w").write(p.stdout)
    p = subprocess.run([TERPD, "tx", "broadcast", sf, "--node", NODE, "-b", "sync", "-o", "json"], capture_output=True, text=True)
    try: res = json.loads(p.stdout)
    except Exception: return {"code": -1, "raw_log": (p.stdout + p.stderr)[-400:]}
    return res if res.get("code", 0) else wait_tx(res["txhash"])

def gov_submit(name, msgs, frm, eph=None):
    """Submit with INIT_DEPOSIT, top up to FULL_DEPOSIT, vote yes (owner = sole validator), wait."""
    f = os.path.join(RUN, "prop_%s.json" % name)
    json.dump({"messages": msgs, "metadata": "", "deposit": "%duterp" % INIT_DEPOSIT, "title": name, "summary": "local gov-roles e2e",
               "expedited": False}, open(f, "w"))
    if eph:
        inner = json.loads(gen_only(frm, "gov", "submit-proposal", f))
        open(os.path.join(RUN, "unsigned_%s.json" % name), "w").write(json.dumps(inner))
        res = sign_send(eph, gs.exec_tx(R["addresses"]["EPH_ADDR"], frm, inner["body"]["messages"], 80000000, SESSION_FEE, "uterp"), "exec_" + name)
    else:
        res = tx(frm, 80000000, "gov", "submit-proposal", f)
    pid = None
    for ev in res.get("events", []):
        for at in ev.get("attributes", []):
            if ev["type"] == "submit_proposal" and at["key"] == "proposal_id": pid = at["value"]
    if not pid: return res, None, None
    R.setdefault("deposits_at_submit", {})[name] = q("gov", "deposits", pid).get("deposits", [])
    dep = tx("proposer", 30000000, "gov", "deposit", pid, "%duterp" % (FULL_DEPOSIT - INIT_DEPOSIT))
    if dep.get("code") != 0: log("deposit top-up failed: %s" % dep.get("raw_log", "")[:300])
    vote = tx("owner", 8000000, "gov", "vote", pid, "yes")
    for _ in range(60):
        time.sleep(1)
        st = q("gov", "proposal", pid); st = st.get("proposal", st)
        if st["status"] not in ("PROPOSAL_STATUS_VOTING_PERIOD", "PROPOSAL_STATUS_DEPOSIT_PERIOD", 1, 2): break
    st["_deposit_code"] = dep.get("code"); st["_vote_code"] = vote.get("code")
    return res, pid, st

def close_session(PROPOSER):
    """Session close via tools/headstash/deploy/gov_session.py: one MsgExec signed by the eph key revokes
    SubmitProposal, the feegrant (MsgRevokeAllowance), its grant, and MsgRevoke last."""
    eph = R["addresses"]["EPH_ADDR"]
    res = sign_send("eph", gs.revoke_tx(eph, PROPOSER, gs.SESSION_TYPES, 400000, 20000, "uterp", use_feegrant=True), "revoke")
    step("session_revoke_msgexec", res.get("code") == 0, "gas %d %s" % (gas_used(res), res.get("raw_log", "")[:160]))
    left = q("authz", "grants", PROPOSER, eph).get("grants", [])
    fg = subprocess.run([TERPD, "q", "feegrant", "grant", PROPOSER, eph, "--node", NODE, "-o", "json"], capture_output=True, text=True)
    step("session_authz_revoked", left == [], "remaining authz %d" % len(left))
    step("session_feegrant_revoked", fg.returncode != 0, (fg.stdout + fg.stderr).strip()[-160:])
    inner = json.load(open(os.path.join(RUN, "unsigned_circuit.json")))
    neg("eph_cannot_submit_after_revoke", sign_send("eph", gs.exec_tx(eph, PROPOSER, inner["body"]["messages"], 80000000, SESSION_FEE, "uterp"), "exec_after_revoke"))

def passed(st): return st is not None and st.get("status") in ("PROPOSAL_STATUS_PASSED", 3)

def main():
    addrs = dict(l.strip().split("=", 1) for l in open(os.path.join(HOME, "addresses.env")) if "=" in l)
    OWNER = addrs["OWNER_ADDR"]
    RELAYER = terpd("keys", "show", "relayer", "-a", "--keyring-backend", "test", "--home", HOME).strip()
    R["mode"] = "direct"
    R["chain"] = {"chain_id": CHAIN_ID, "rpc": RPC, "rest": REST, "relay": RELAY_URL, "start_height": height()}
    R["addresses"] = dict(addrs, RELAYER_ADDR=RELAYER)

    # 1. identities + note eligibility (EVM 200008 TERP -> 43 notes, Sol 3.5 TERP -> 2 notes)
    evm_priv = secrets.randbelow(N - 1) + 1
    pt = pmul(evm_priv)
    evm_addr = kec(pt[0].to_bytes(32, "big") + pt[1].to_bytes(32, "big"))[12:]
    sol_sk = Ed25519PrivateKey.generate()
    sol_pk = sol_sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    ids = {"evm": (0x01, evm_addr, evm_addr.hex()), "sol": (0x02, sol_pk, "sol:" + sol_pk.hex())}
    comm = os.path.join(RUN, "communities", "mvk"); os.makedirs(comm, exist_ok=True)
    open(os.path.join(comm, "mvk.csv"), "w").write("addr,amount\n0x%s,200008\n%s,3.5\n" % (evm_addr.hex(), b58(sol_pk)))
    elig = os.path.join(RUN, "eligibility")
    subprocess.run([ELIG_BIN, os.path.join(RUN, "communities"), elig, "--decimals", "mvk=6", "--denominations"], check=True, capture_output=True)
    ntree = json.load(open(os.path.join(elig, "notes", "tree.json")))
    paths = [json.loads(l) for l in open(os.path.join(elig, "notes", "note_paths.jsonl"))]
    notes = {k: sorted([p for p in paths if p["identity"] == ids[k][2]], key=lambda p: p["note_index"]) for k in ids}
    step("note_eligibility_tree", ntree["depth"] == 17 and {k: len(v) for k, v in notes.items()} == {"evm": 43, "sol": 2},
         "root %s notes %s" % (ntree["root_hex"], ntree["notes"]))

    # 2. circuit store (vk_id 3 blob split params || vk_body)
    cdir = os.path.join(RUN, "circuit"); os.makedirs(cdir, exist_ok=True)
    subprocess.run([NOTE_EX + "/bench_note_proofs", cdir, "0"], check=True, capture_output=True)
    blob = open(os.path.join(cdir, "blob.bin"), "rb").read()
    step("circuit_blob_hash", hashlib.sha256(blob).hexdigest() == NOTE_VK_HASH, "blob %d B sha %s" % (len(blob), hashlib.sha256(blob).hexdigest()))
    if GOV_ROLES:
        PROPOSER = terpd("keys", "show", "proposer", "-a", "--keyring-backend", "test", "--home", HOME).strip()
        R["addresses"]["PROPOSER_ADDR"] = PROPOSER
        res = tx("stranger", 60000000, "wasm", "store-full-circuit", os.path.join(cdir, "params.bin"),
                 os.path.join(cdir, "vk_body.bin"), "--k", "11", "--circuit-type", "0", "--curve-type", "0")
        neg("direct_circuit_store_refused_nobody", res)
        # cw-orch-kms session shape: cold key grants a short GenericAuthorization + feegrant to an ephemeral key
        EPH = new_wallet("eph"); R["addresses"]["EPH_ADDR"] = EPH
        exp = int(time.time()) + 600
        exp_rfc = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(exp))
        parts = [json.loads(gen_only("proposer", "authz", "grant", EPH, "generic", "--msg-type", url, "--expiration", str(exp))) for url in gs.SESSION_TYPES]
        parts.append(json.loads(gen_only("proposer", "feegrant", "grant", PROPOSER, EPH, "--spend-limit", "%duterp" % SESSION_ALLOWANCE, "--expiration", exp_rfc)))
        gtx = parts[0]; gtx["body"]["messages"] = [m for p_ in parts for m in p_["body"]["messages"]]; gtx["body"]["memo"] = "cw-orch-session:v1"
        gtx["auth_info"]["fee"] = {"amount": [{"denom": "uterp", "amount": "20000"}], "gas_limit": "600000", "payer": "", "granter": ""}
        f_ = sign_send("proposer", gtx, "session_grant", mode="amino-json")
        step("session_grants_amino_signed", f_.get("code") == 0, "one amino tx: generic %s + feegrant %d%s exp %s %s" % (
            ",".join(u.rsplit(".", 1)[1] for u in gs.SESSION_TYPES), SESSION_ALLOWANCE, "uterp", exp_rfc, f_.get("raw_log", "")[:120]))
        b0, e0 = bal(PROPOSER), bal(EPH)
        msg = {"@type": "/cosmwasm.wasm.v1.MsgStoreFullCircuit", "sender": GOV, "auth": {"circuit_type": 0, "curve_type": 0, "k": 11},
               "params_body": base64.b64encode(open(os.path.join(cdir, "params.bin"), "rb").read()).decode(),
               "vk_body": base64.b64encode(open(os.path.join(cdir, "vk_body.bin"), "rb").read()).decode()}
        res, pid, st = gov_submit("circuit", [msg], PROPOSER, eph="eph")
        res = rec("store_circuit", res)
        R["circuit_proposal"] = {"id": pid, "status": st and st["status"], "tx_bytes_unsigned": os.path.getsize(os.path.join(RUN, "unsigned_circuit.json"))}
        step("eph_msgexec_submit_proposal", res.get("code") == 0 and pid is not None, "proposal %s gas %d %s" % (pid, gas_used(res), res.get("raw_log", "")[:160]))
        if pid:
            pp = q("gov", "proposal", pid); pp = pp.get("proposal", pp)
            step("proposer_is_granter", pp.get("proposer") == PROPOSER, "proposer %s" % pp.get("proposer"))
            deps = R["deposits_at_submit"]["circuit"]  # queried right after submit, before top-up/refund
            tot = sum(int(c["amount"]) for d in deps for c in d["amount"] if c["denom"] == "uterp")
            step("initial_deposit_by_granter", tot == INIT_DEPOSIT and len(deps) > 0 and all(d["depositor"] == PROPOSER for d in deps), "at submit %s" % json.dumps(deps)[:200])
            tot = sum(int(c["amount"]) for c in pp.get("total_deposit", []) if c["denom"] == "uterp")  # deposits are refunded+pruned once passed; total_deposit stays
            step("deposit_from_granter", tot == FULL_DEPOSIT and all(d["depositor"] == PROPOSER for d in deps) and b0 - bal(PROPOSER) > 0 and bal(EPH) == e0 == 0,
                 "deposits %s proposer -%d eph %d" % (json.dumps(deps)[:200], b0 - bal(PROPOSER), bal(EPH)))
        step("circuit_proposal_passed", passed(st), "status %s %s" % (st and st["status"], st and st.get("failed_reason", "")))
        # wasm stores (> 64 KiB, not Zigner-signable): the eph key stores as itself while the session is open
        R["eph_code_id"], _ = eph_store("cw_headstash", WASM, PROPOSER, 20000000)
        if FE_GOV:
            R["fe_code_id"], R["fe_checksum"] = eph_store("frontend", FE_GOV["wasm"], PROPOSER, 8000000)
        close_session(PROPOSER)  # close right after the large action, well inside the TTL
    else:
        res = rec("store_circuit", tx("owner", 60000000, "wasm", "store-full-circuit", os.path.join(cdir, "params.bin"),
              os.path.join(cdir, "vk_body.bin"), "--k", "11", "--circuit-type", "0", "--curve-type", "0"))
    zk_id = None
    for ev in res.get("events", []):
        for at in ev.get("attributes", []):
            if at["key"] in ("zk_id", "circuit_id", "zkid", "id") and at["value"].isdigit() and zk_id is None and "circuit" in ev["type"]:
                zk_id = int(at["value"])
    R["circuit_store_events"] = [ev for ev in res.get("events", []) if "circuit" in ev.get("type", "")]
    zk_id = zk_id or int(os.environ.get("ZK_ID", "1"))
    R["zk_id"] = zk_id; R["gas"] = {"store_circuit": gas_used(res)}
    if not GOV_ROLES: step("store_circuit", res.get("code") == 0, "zk_id %s gas %d %s" % (zk_id, gas_used(res), res.get("raw_log", "")[:160]))

    # 3. store + instantiate the zk-api contract
    R["wasm_sha256"] = hashlib.sha256(open(WASM, "rb").read()).hexdigest()
    if GOV_ROLES and R.get("eph_code_id"):
        code_id = R["eph_code_id"]; R["code_id"] = code_id
    else:
        res = rec("store", tx("owner", 250000000, "wasm", "store", WASM))
        code_id = attr(res, "code_id"); R["code_id"] = code_id
        step("store_wasm", res.get("code") == 0 and code_id, "code_id %s wasm %s gas %d" % (code_id, R["wasm_sha256"], gas_used(res)))
    # WAVS_JSON: use a fixed public set (e.g. the rendered mainnet one) instead of a fresh print_wavs set
    if os.environ.get("WAVS_JSON"): wavs = json.load(open(os.environ["WAVS_JSON"]))
    else: wavs = json.loads(subprocess.run([WAVS_BIN], capture_output=True, text=True, check=True).stdout)
    R["wavs_aggregate_key"] = wavs["msg"]["aggregate_key"]
    init = {"genesis_root": b64(bytes(32)), "genesis_label": "headstash-direct-e2e",
            "token_strategy": {"existing_fungible": {"proof": h2b64(DENOM), "raw": "uterp"}}, "wavs": wavs}
    R["hook"] = {"enabled": HOOK}
    if HOOK:
        res = rec("store_hook", tx("owner", 20000000, "wasm", "store", HOOK_WASM))
        hcode = attr(res, "code_id")
        res = rec("instantiate_hook", tx("owner", 2000000, "wasm", "instantiate", hcode, json.dumps({"source": None}),
                  "--label", "hook-recorder-e2e", "--no-admin"))
        hook = attr(res, "_contract_address")
        step("hook_recorder_instantiated", res.get("code") == 0 and hook, "code %s addr %s" % (hcode, hook))
        R["hook"].update({"code_id": hcode, "contract": hook, "wasm_sha256": hashlib.sha256(open(HOOK_WASM, "rb").read()).hexdigest()})
        init["registration_hook"] = hook
        bad = dict(init, registration_hook="terp1notanaddress")
        neg("instantiate_rejects_bad_hook", tx("owner", 80000000, "wasm", "instantiate", code_id, json.dumps(bad),
            "--label", "bad-hook", "--admin", OWNER, amount="1uterp"), "")
        b.HOOK_ADDR = hook
    for k in ("evm", "sol"):
        REWARD[k] = new_wallet("reward_%s" % k)
    R["hook"]["reward_addrs"] = dict(REWARD)
    if GOV_ROLES:
        salt = hashlib.sha256(b"headstash-gov-e2e").hexdigest()
        chk = q("wasm", "code-info", code_id); chk = chk.get("checksum") or chk.get("data_hash")
        contract = terpd("q", "wasm", "build-address", chk.lower(), GOV, salt).strip().split()[-1]
        if FE_GOV:
            R["fe_contract"] = terpd("q", "wasm", "build-address", R["fe_checksum"], GOV, FE_SALT).strip().split()[-1]
            init["frontend_contract"] = R["fe_contract"]
    else:
        res = rec("instantiate", tx("owner", 80000000, "wasm", "instantiate", code_id, json.dumps(init),
              "--label", "headstash-direct-e2e", "--admin", OWNER, amount="10030000000uterp"))  # two 5000 TERP note payouts (evm_0 + negatives note) + small notes
        contract = attr(res, "_contract_address")
    R["contract"] = contract
    if not GOV_ROLES: step("instantiate", res.get("code") == 0 and contract, contract or res.get("raw_log", ""))
    b.R = R

    ctx = direct_ctx(CHAIN_ID, contract); R["ctx_hex"] = ctx.hex()
    cut = height() + CUT_AHEAD; R["cutoff_height"] = cut
    classes = {}
    for k in ids:
        for p in notes[k]: classes[p["class"]] = classes.get(p["class"], 0) + 1
    amounts = [{"amount": DENOMS[c], "cap": n} for c, n in sorted(classes.items())]
    def campaign(c=ctx):
        return {"set_epoch_campaign": {"campaign": {
            "ctx": b64(c), "backend": "halo2", "cutoff_height": cut, "bech32_hrp": "terp", "denom": h2b64(DENOM),
            "amounts": amounts, "reserved": [], "trusted": [], "trusted_verify_threshold": 0, "trusted_sign_threshold": 0,
            "max_rows": 8, "allocation_root": b64(bytes(32)), "root_id": 0, "vk_id": 3, "vk_hash": h2b64(NOTE_VK_HASH),
            "sweep_recipient": "",  # vk_id 3 burns unclaimed by default (burn_unclaimed omitted)
            "registration": {"eligibility_root": h2b64(ntree["root_hex"]), "eligibility_depth": 17,
                             "min_anonymity_set": 2, "anchor_start_height": 1, "anchor_interval": 1}}}}
    eff = campaign()
    if GOV_ROLES:
        # one small proposal: instantiate2 (gov admin+owner) + campaign + bind + community-pool funding
        cc = eff["set_epoch_campaign"]["campaign"]  # contract requires a non-empty trusted set; 1-of-1 placeholder, unused on the direct path
        cc["trusted"] = [b64(bytes(32))]; cc["trusted_verify_threshold"] = 1; cc["trusted_sign_threshold"] = 1
        res = tx("owner", 300000, "distribution", "fund-community-pool", "10100000000uterp")
        step("community_pool_funded", res.get("code") == 0, res.get("raw_log", "")[:120])
        msgs = [{"@type": "/cosmos.distribution.v1beta1.MsgCommunityPoolSpend", "authority": GOV, "recipient": GOV,
                 "amount": [{"denom": "uterp", "amount": "10030000000"}]},  # pool -> gov, then gov funds the instantiate
                {"@type": "/cosmwasm.wasm.v1.MsgInstantiateContract2", "sender": GOV, "admin": GOV, "code_id": str(code_id),
                 "label": "headstash-direct-e2e", "msg": init, "funds": [{"denom": "uterp", "amount": "10030000000"}], "salt": base64.b64encode(bytes.fromhex(salt)).decode(), "fix_msg": False},
                {"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": GOV, "contract": contract, "msg": eff, "funds": []},
                {"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": GOV, "contract": contract, "msg": {"bind_note_circuit": {"zk_id": zk_id}}, "funds": []},
]
        if FE_GOV:  # frontend first: instantiate2 (gov admin + owner) at the predicted address, then publish the manifest (activate)
            fe = R["fe_contract"]; R["fe_manifest"] = FE_GOV["manifest"]()
            msgs = [{"@type": "/cosmwasm.wasm.v1.MsgInstantiateContract2", "sender": GOV, "admin": GOV, "code_id": str(R["fe_code_id"]),
                     "label": "headstash-frontend-gov-e2e", "msg": {"owner": GOV, "max_chunk_bytes": FE_GOV["max_chunk_bytes"]}, "funds": [],
                     "salt": base64.b64encode(bytes.fromhex(FE_SALT)).decode(), "fix_msg": False},
                    {"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": GOV, "contract": fe,
                     "msg": {"publish_manifest": {"manifest": R["fe_manifest"]}}, "funds": []}] + msgs
        res, pid, st = gov_submit("contract", msgs, "proposer")
        R["contract_proposal"] = {"id": pid, "status": st and st["status"], "failed_reason": st and st.get("failed_reason")}
        ci = q("wasm", "contract", contract) if passed(st) else {}
        ci = ci.get("contract_info", ci)
        step("contract_proposal_passed", passed(st), "proposal %s %s %s" % (pid, st and st["status"], st and st.get("failed_reason", "")))
        step("instantiate", passed(st) and ci.get("admin") == GOV and ci.get("creator") == GOV and bal(contract) == 10030000000,
             "%s admin %s creator %s bal %d" % (contract, ci.get("admin"), ci.get("creator"), bal(contract) if passed(st) else -1))
        if not passed(st): return
        info = smart(contract, {"direct_claim": {}})
        step("direct_ctx_matches_contract", base64.b64decode(info["expected_ctx"]) == ctx, json.dumps(info)[:200])
        step("store_circuit_and_bind_via_gov", True, "zk_id %s bound by gov proposal (bind checks sha256(circuit)==vk_hash)" % zk_id)
        res = tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id}}))
        neg("owner_key_is_not_contract_owner", res)
        res = tx("stranger", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id}}))
        neg("bind_by_non_owner", res)
    else:
      info = smart(contract, {"direct_claim": {}})
      step("direct_ctx_matches_contract", base64.b64decode(info["expected_ctx"]) == ctx, json.dumps(info)[:200])
      res = rec("set_campaign", tx("owner", 5000000, "wasm", "execute", contract, json.dumps(eff)))
      if res.get("code") != 0:  # trusted set may be mandatory; fall back to a 1-of-1 placeholder (unused by the direct path)
          c = campaign(); cc = c["set_epoch_campaign"]["campaign"]
          cc["trusted"] = [b64(bytes(32))]; cc["trusted_verify_threshold"] = 1; cc["trusted_sign_threshold"] = 1
          R["set_campaign_first_error"] = res.get("raw_log", "")[:300]
          res = rec("set_campaign", tx("owner", 5000000, "wasm", "execute", contract, json.dumps(c))); eff = c
      step("set_campaign_vk3_direct_ctx", res.get("code") == 0, res.get("raw_log", "")[:200])
      # NEG: the same (accepted) campaign with a sweep recipient is refused: vk_id 3 burns unclaimed by default
      cr = json.loads(json.dumps(eff)); cr["set_epoch_campaign"]["campaign"]["sweep_recipient"] = OWNER
      neg("burning_campaign_rejects_sweep_recipient", tx("owner", 5000000, "wasm", "execute", contract, json.dumps(cr)),
          "takes no sweep recipient")

      # 4. BindNoteCircuit: negatives then the real bind
      res = tx("stranger", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id}}))
      neg("bind_by_non_owner", res)
      res = tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id + 99}}))
      neg("wrong_zk_id_unknown_circuit", res)
      res = rec("bind_circuit", tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id}})))
      R["gas"]["bind_circuit"] = gas_used(res)
      step("bind_note_circuit_hash_matches", res.get("code") == 0,
           "sha256(Circuit.data) == vk_hash on chain: %s; %s" % (res.get("code") == 0, res.get("raw_log", "")[:240]))
      if res.get("code") != 0: return
      res = tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id}}))
      neg("rebind_zk_id_swap", res, "circuit is fixed")

    # 5. relay (fee grants) on 127.0.0.1:18795
    env = dict(os.environ, HEADSTASH_RELAY_HOME=HOME, HEADSTASH_RELAY_FROM="relayer", HEADSTASH_KEYRING_BACKEND="test",
               HEADSTASH_REST=REST, HEADSTASH_CONTRACT=contract, HEADSTASH_CHAIN_ID=CHAIN_ID, HEADSTASH_RELAY_BIND=RELAY_BIND,
               HEADSTASH_GAS_PRICE="0.025uterp", HEADSTASH_CLAIM_GAS=str(CLAIM_GAS), HEADSTASH_REGISTER_GAS=str(REGISTER_GAS),
               TERPD=TERPD, PATH="/usr/local/bin:" + os.environ.get("PATH", ""))
    env.pop("HEADSTASH_ELIGIBILITY", None)
    rlog = open(os.path.join(RUN, "relay.log"), "w")
    relay = subprocess.Popen([RELAY_BIN], env=env, stdout=rlog, stderr=subprocess.STDOUT)
    open(os.path.join(RUN, "relay.pid"), "w").write(str(relay.pid))
    try:
        up = False
        for _ in range(60):
            time.sleep(1)
            if "relay_up" in open(os.path.join(RUN, "relay.log")).read(): up = True; break
            if relay.poll() is not None: break
        step("relay_up", up, open(os.path.join(RUN, "relay.log")).read()[-300:])
        if not up: return
        flow(contract, ctx, ids, notes, evm_priv, sol_sk, sol_pk, evm_addr, RELAYER, OWNER, cut)
    finally:
        relay.terminate()
        try: relay.wait(10)
        except Exception: relay.kill()

def flow(contract, ctx, ids, notes, evm_priv, sol_sk, sol_pk, evm_addr, RELAYER, OWNER, cut):
    claim_spend = int(CLAIM_GAS * GP + 0.999999); reg_spend = int(REGISTER_GAS * GP + 0.999999)
    R["spend"] = {"claim": claim_spend, "register": reg_spend, "claim_gas_limit": CLAIM_GAS, "register_gas_limit": REGISTER_GAS}
    sec = {k: [pallas_rand() for _ in notes[k]] for k in ids}
    kcs = {k: [subprocess.run([EX + "/membership_key_com"], input=s.hex(), capture_output=True, text=True, check=True).stdout.strip()
               for s in sec[k]] for k in ids}
    lists = {k: [(p["class"], kcs[k][i]) for i, p in enumerate(notes[k])] for k in ids}
    def wire(lst): return [{"class": c, "key_com": h2b64(kc)} for c, kc in lst]
    def signer(k): return {"evm": {"address": b64(evm_addr)}} if k == "evm" else {"sol": {"pubkey": b64(sol_pk)}}
    def sign(k, lst):
        lh = list_hash(ctx, ids[k][0], ids[k][1], lst).hex()
        if k == "evm":
            return evm_sign(evm_priv, eip191("headstash-register-notes-v3:%s:%s:%d:%s" % (ctx.hex(), lh, len(lst), REWARD[k])))
        return sol_sk.sign(("headstash-register-notes-sol-v3:%s:%s:%d:%s" % (ctx.hex(), lh, len(lst), REWARD[k])).encode())
    def reg_body(k, lst, start, sig, rows, reward=None):
        return {"signer": signer(k), "signature": b64(sig) if sig else None, "list": wire(lst), "start": start,
                "eligibility": [{"index": r["leaf_index"], "path": [h2b64(x) for x in r["path"]]} for r in rows],
                "reward_addr": (reward or REWARD[k]) if sig else None}

    def granted_register(label, k, lst, start, sig, rows):
        w = new_wallet("dreg_%s" % label)
        body = reg_body(k, lst, start, sig, rows)
        code, j = grant(w, "register", body)
        step("grant_register_%s" % label, code == 200 and j.get("granted"), json.dumps(j)[:240])
        if code != 200: return None
        res = rec("register_%s" % label, txg("dreg_%s" % label, REGISTER_GAS, reg_spend, RELAYER, "wasm", "execute", contract,
                                           json.dumps({"register_notes": body})))
        R["gas"]["register_%s" % label] = {"notes": len(rows), "gas_used": gas_used(res)}
        step("register_notes_%s" % label, res.get("code") == 0, "%d notes gas %d %s" % (len(rows), gas_used(res), res.get("raw_log", "")[:160]))
        step("register_wallet_unfunded_%s" % label, bal(w) == 0, "fee paid by granter; wallet balance %d" % bal(w))
        return res

    # NEG: grant refused for a bad registration signature
    w = new_wallet("dreg_bad")
    badsig = bytearray(sign("evm", lists["evm"])); badsig[5] ^= 1
    code, j = grant(w, "register", reg_body("evm", lists["evm"], 0, bytes(badsig), notes["evm"][:CHUNK]))
    R["negatives"]["grant_refused_bad_register_sig"] = "%d %s" % (code, json.dumps(j)[:200])
    step("neg_grant_refused_bad_register_sig", code != 200 and not j.get("granted"), R["negatives"]["grant_refused_bad_register_sig"])

    # NEG: relayer swaps the signed reward_addr (v3 signature covers it); malformed reward_addr
    sig = sign("evm", lists["evm"])
    res = tx("stranger", REGISTER_GAS, "wasm", "execute", contract,
             json.dumps({"register_notes": reg_body("evm", lists["evm"], 0, sig, notes["evm"][:CHUNK], reward=REWARD["sol"])}))
    R["negatives"]["tampered_reward_addr"] = "code %s %s" % (res.get("code"), res.get("raw_log", "")[:200])
    step("neg_tampered_reward_addr", res.get("code") not in (0, None), R["negatives"]["tampered_reward_addr"])
    res = tx("stranger", REGISTER_GAS, "wasm", "execute", contract,
             json.dumps({"register_notes": reg_body("evm", lists["evm"], 0, sig, notes["evm"][:CHUNK], reward="cosmos1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqnrql8a")}))
    neg("bad_reward_addr_prefix", res, "reward_addr")

    # 6. EVM registration in two chunks (32 + 11), each from a fresh fee-granted wallet
    r0 = granted_register("evm0", "evm", lists["evm"], 0, sig, notes["evm"][:CHUNK])
    if not r0: return
    r1 = granted_register("evm1", "evm", lists["evm"], CHUNK, None, notes["evm"][CHUNK:])
    if not r1: return
    if HOOK:
        hook = R["hook"]["contract"]
        regs = smart(hook, {"registrants": {}})
        R["hook"]["after_evm"] = regs
        step("hook_not_fired_on_partial_chunk", attr(r0, "registration_hook") is None, "evm0 attrs ok")
        step("hook_recorded_registrant_and_reward_addr",
             len(regs) == 1 and regs[0]["identity"] == evm_addr.hex() and regs[0]["reward_addr"] == REWARD["evm"]
             and regs[0]["note_count"] == 43 and regs[0]["caller"] == contract and regs[0]["funds"] == 0
             and regs[0]["campaign"] == ctx.hex() and regs[0]["id_tag"] == 1, json.dumps(regs)[:300])
        res = rec("hook_set_fail", tx("owner", 500000, "wasm", "execute", hook, json.dumps({"set_fail": {"fail": True}})))
        step("hook_set_fail", res.get("code") == 0, res.get("raw_log", "")[:120])

    # 7. anchor A1 (43 EVM notes)
    def anchor_now(label, want):
        rt = smart(contract, {"registration_tree": {}})
        while height() < int(rt.get("next_anchor_height", 0)): time.sleep(1)
        res = rec("accept_anchor_" + label, tx("stranger", 5000000, "wasm", "execute", contract, json.dumps({"accept_anchor": {}})))
        rt = smart(contract, {"registration_tree": {}})
        for _ in range(15):
            if rt.get("latest_anchor"): break
            time.sleep(1); rt = smart(contract, {"registration_tree": {}})
        if not rt.get("latest_anchor"):
            raise RuntimeError("no latest_anchor after accept_anchor_%s: %s" % (label, json.dumps(rt)[:400]))
        a = rt["latest_anchor"]
        step("accept_anchor_" + label, res.get("code") == 0 and a["leaf_count"] == want,
             "root %s leaf_count %s height %s gas %d" % (base64.b64decode(a["root"]).hex(), a["leaf_count"], a.get("height"), gas_used(res)))
        R["gas"]["accept_anchor_" + label] = gas_used(res)
        return base64.b64decode(a["root"]), rt
    A1, _ = anchor_now("A1", 43)

    # 8. Sol notes registered after A1
    rs = granted_register("sol", "sol", lists["sol"], 0, sign("sol", lists["sol"]), notes["sol"])
    if not rs: return
    if HOOK:
        regs = smart(R["hook"]["contract"], {"registrants": {}})
        R["hook"]["after_sol"] = regs
        failed = [a.get("value") for ev in rs.get("events", []) for a in ev.get("attributes", []) if a.get("key") == "hook_error"]
        step("failing_hook_does_not_block_registration",
             rs.get("code") == 0 and attr(rs, "action") is not None and len(regs) == 1 and len(failed) == 1
             and any(a.get("value") == "registration_hook_failed" for ev in rs.get("events", []) for a in ev.get("attributes", [])),
             "registrants %d hook_error %s" % (len(regs), failed[:1]))
    rt = smart(contract, {"registration_tree": {}})
    live = base64.b64decode(rt["root"])

    def witness(k, i, dest, root, amount=None):
        pth = smart(contract, {"registration_path": {"key_com": h2b64(kcs[k][i]), "anchor": b64(root)}})
        return json.dumps({"secret_hex": sec[k][i].hex(), "ctx_hex": ctx.hex(), "root_hex": root.hex(), "destination": dest,
                           "amount": str(amount if amount is not None else pth["amount"]), "index": pth["index"],
                           "siblings_hex": pth["siblings"]}), pth
    def prove(k, i, dest, root, amount=None):
        wit, pth = witness(k, i, dest, root, amount)
        t0 = time.time()
        out = subprocess.run([NOTE_EX + "/prove_note_witness", "/dev/stdin"], input=wit, capture_output=True, text=True)
        if out.returncode != 0: return None, out.stderr.strip()[-200:]
        pr = json.loads(out.stdout); p = pr["body"].split(":")  # headstash-note-claim-v2:ctx:root:nullifier:dest:amount
        pr.update({"root": b64(root), "nullifier": h2b64(p[3]), "amount": int(p[5]), "dest": p[4],
                   "proof": h2b64(pr["proof_hex"]), "secs": round(time.time() - t0, 1)})
        return pr, ""
    def claim_msg(pr, **o):
        m = {"root": pr["root"], "nullifier": pr["nullifier"], "amount": pr["amount"], "proof": pr["proof"]}; m.update(o)
        return {"claim_note": m}

    # NEG: a note registered after A1 has no path in A1 (cannot claim against A1)
    try:
        smart(contract, {"registration_path": {"key_com": h2b64(kcs["sol"][0]), "anchor": b64(A1)}}); r_ = "path returned"
    except Exception as e: r_ = str(e)[-200:]
    R["negatives"]["late_note_not_in_older_anchor"] = r_
    step("neg_late_note_not_in_older_anchor", r_ != "path returned", r_)

    # NEG: live root (not an accepted anchor) -> grant refused and tx rejected. The contract serves
    # paths only under accepted anchors, so the live-root claim carries a real proof (evm_1 under A1)
    # with root = live; the anchor check runs before verify. After A2 snapshots the same root, a
    # proof under it is accepted (live_root_becomes_claimable_once_anchored).
    wl = new_wallet("dclaim_live")
    prl, err = prove("evm", 1, wl, A1)
    if prl:
        code, j = grant(wl, "claim", claim_msg(prl, root=b64(live))["claim_note"])
        res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prl, root=b64(live))))
        R["negatives"]["live_root_grant"] = "%d %s" % (code, json.dumps(j)[:160])
        neg("claim_against_live_root", res, "not an accepted anchor")
        step("neg_grant_refused_live_root", code != 200, R["negatives"]["live_root_grant"])
    else:
        step("prove_live_root", False, err)
    # NEG: fabricated root
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prl, root=b64(pallas_rand()))))
    neg("claim_against_fabricated_root", res, "not an accepted anchor")

    A2, rt2 = anchor_now("A2", 45)
    R["anchors"] = [{"root_hex": base64.b64decode(a["root"]).hex(), "leaf_count": a["leaf_count"], "height": a.get("height")} for a in all_anchors(contract)]
    step("anchor_map_paged", rt2["anchor_count"] >= 2 and len(R["anchors"]) == rt2["anchor_count"] and "anchors" not in rt2, "anchor map entries %d (paged registration_anchors)" % rt2["anchor_count"])

    # NEG: a note registered after A1 cannot claim against A1 (forced root swap on a valid A2 proof)
    ws = new_wallet("dclaim_late")
    prs, err = prove("sol", 0, ws, A2)
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prs, root=b64(A1))))
    neg("late_note_claim_against_older_anchor", res, "invalid note proof")

    # 9. positive claims: one fresh fee-granted wallet per note
    step("live_root_becomes_claimable_once_anchored", A2 == live, "live root at rejection %s == A2 %s" % (live.hex()[:16], A2.hex()[:16]))
    picks = [("evm", 0, A2), ("evm", 41, A1), ("evm", 42, A2), ("sol", 1, A2)]
    R["claims"] = []
    granter0 = bal(RELAYER)
    for j_, (k, i, root) in enumerate(picks):
        name = "dclaim%d" % j_
        wlt = new_wallet(name)
        pr, err = prove(k, i, wlt, root)
        if not pr: step("prove_%s_%d" % (k, i), False, err); return
        assert pr["dest"] == wlt
        c0, g0 = bal(contract), bal(RELAYER)
        code, j = grant(wlt, "claim", claim_msg(pr)["claim_note"])
        step("grant_claim_%s_%d" % (k, i), code == 200 and j.get("granted"), json.dumps(j)[:200])
        if code != 200: return
        g1 = bal(RELAYER)
        res = rec("claim_%s_%d" % (k, i), txg(name, CLAIM_GAS, claim_spend, RELAYER, "wasm", "execute", contract, json.dumps(claim_msg(pr))))
        c1, g2, w1 = bal(contract), bal(RELAYER), bal(wlt)
        ok = res.get("code") == 0 and w1 == pr["amount"] and c0 - c1 == pr["amount"] and g1 - g2 == claim_spend
        R["claims"].append({"note": "%s_%d" % (k, i), "anchor": "A1" if root == A1 else "A2", "wallet": wlt, "amount": pr["amount"],
                            "gas_used": gas_used(res), "gas_wanted": int(res.get("gas_wanted", 0) or 0), "fee": claim_spend,
                            "grant_tx_fee": g0 - g1, "prove_secs": pr["secs"], "proof_bytes": pr["proof_bytes"]})
        step("direct_claim_%s_%d%s" % (k, i, "_older_anchor_A1" if root == A1 else ""), ok,
             "paid %d to fresh wallet (bal %d) contract -%d granter -%d (fee) gas %d" % (pr["amount"], w1, c0 - c1, g1 - g2, gas_used(res)))
        if j_ == 0: first = (name, wlt, pr, k, i)
    gs = [c["gas_used"] for c in R["claims"]]
    R["gas"]["claim"] = {"min": min(gs), "max": max(gs), "mean": sum(gs) // len(gs)}
    R["gas"]["grant_tx_fee"] = [c["grant_tx_fee"] for c in R["claims"]]
    st = smart(contract, {"direct_claim": {}})
    step("direct_claim_counter", st.get("claimed") == len(picks), json.dumps(st)[:200])

    # 10. negatives against real proofs
    name0, w0, pr0, k0, i0 = first
    # allowance is pruned after its one fee
    res = txg(name0, CLAIM_GAS, claim_spend, RELAYER, "wasm", "execute", contract, json.dumps({"accept_anchor": {}}))
    neg("allowance_single_use", res)
    # replayed nullifier: fresh proof of an already claimed note to a new wallet
    wr = new_wallet("dclaim_replay")
    prr, err = prove(k0, i0, wr, A2)
    code, j = grant(wr, "claim", claim_msg(prr)["claim_note"])
    R["negatives"]["grant_refused_replayed_nullifier"] = "%d %s" % (code, json.dumps(j)[:200])
    step("neg_grant_refused_replayed_nullifier", code != 200, R["negatives"]["grant_refused_replayed_nullifier"])
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prr)))
    neg("replayed_nullifier", res, "already")
    # fresh unclaimed note for the remaining negatives
    wn = new_wallet("dclaim_neg")
    paid_n = {}
    for c in R["claims"]: paid_n[int(c["amount"])] = paid_n.get(int(c["amount"]), 0) + 1
    camp_amounts = smart(contract, {"epoch_campaign": {}})["amounts"]
    room = lambda amt: next(int(a["cap"]) for a in camp_amounts if int(a["amount"]) == int(amt)) - paid_n.get(int(amt), 0)
    used = {(c["note"].split("_")[0], int(c["note"].split("_")[1])) for c in R["claims"]}
    ni = next(i for i in range(len(notes["evm"])) if ("evm", i) not in used and i != 1 and room(DENOMS[notes["evm"][i]["class"]]) >= 1)
    R["neg_note"] = "evm_%d" % ni
    prn, err = prove("evm", ni, wn, A2)
    forged = bytearray(base64.b64decode(prn["proof"])); forged[100] ^= 1
    code, j = grant(wn, "claim", claim_msg(prn, proof=b64(bytes(forged)))["claim_note"])
    R["negatives"]["grant_refused_forged_proof"] = "%d %s" % (code, json.dumps(j)[:200])
    step("neg_grant_refused_invalid_proof", code != 200 and not j.get("granted"), R["negatives"]["grant_refused_forged_proof"])
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn, proof=b64(bytes(forged)))))
    neg("forged_proof", res, "invalid note proof")
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn, root=b64(A1))))
    neg("wrong_root_other_anchor", res, "invalid note proof")
    other = [a["amount"] for a in json.loads(json.dumps(smart(contract, {"epoch_campaign": {}})["amounts"])) if a["amount"] != prn["amount"] and room(a["amount"]) >= 1][0]
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn, amount=other)))
    neg("wrong_amount_same_set_swap", res, "invalid note proof")
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn, amount=prn["amount"] + 1)))
    neg("wrong_amount_not_denomination", res)
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn)))
    neg("wrong_sender_dest", res, "invalid note proof")
    # grant scope: a granted wallet cannot spend the allowance on another message type
    code, j = grant(wn, "claim", claim_msg(prn)["claim_note"])
    step("grant_claim_neg_wallet", code == 200 and j.get("granted"), json.dumps(j)[:200])
    res = txg("dclaim_neg", 200000, claim_spend, RELAYER, "bank", "send", wn, OWNER, "1uterp")
    neg("allowance_rejects_bank_send", res)
    res = txg("dclaim_neg", CLAIM_GAS, claim_spend + 1, RELAYER, "wasm", "execute", contract, json.dumps(claim_msg(prn)))
    neg("allowance_rejects_fee_above_spend_limit", res)
    code, j = grant(wn, "claim", claim_msg(prn)["claim_note"])
    R["negatives"]["grant_second_for_same_nullifier"] = "%d %s" % (code, json.dumps(j)[:200])
    step("neg_grant_dedupe_same_nullifier", code != 200, R["negatives"]["grant_second_for_same_nullifier"])
    c0 = bal(contract)
    res = rec("claim_neg_wallet_ok", txg("dclaim_neg", CLAIM_GAS, claim_spend, RELAYER, "wasm", "execute", contract, json.dumps(claim_msg(prn))))
    step("direct_claim_after_negatives", res.get("code") == 0 and bal(wn) == prn["amount"] and c0 - bal(contract) == prn["amount"],
         "gas %d" % gas_used(res))
    R["claims"].append({"note": R["neg_note"], "anchor": "A2", "wallet": wn, "amount": prn["amount"], "gas_used": gas_used(res), "fee": claim_spend})
    save()

    # 11. window close + sweep
    if SWEEP:
        log("waiting for cutoff", cut, "now", height())
        while height() <= cut + 1: time.sleep(2)
        wc = new_wallet("dclaim_late2")
        prc, err = prove("sol", 0, wc, A2)
        code, j = grant(wc, "claim", claim_msg(prc)["claim_note"])
        step("neg_grant_refused_after_cutoff", code != 200 and "window" in json.dumps(j) or code != 200, "%d %s" % (code, json.dumps(j)[:160]))
        res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prc)))
        neg("claim_after_cutoff", res, "claim window closed")
        # registration closes at cutoff: a correctly signed body (sol list) now fails before any other check
        sbody = reg_body("sol", lists["sol"], 0, sign("sol", lists["sol"]), notes["sol"])
        wr2 = new_wallet("dreg_late")
        code, j = grant(wr2, "register", sbody)
        R["negatives"]["grant_refused_register_after_cutoff"] = "%d %s" % (code, json.dumps(j)[:200])
        step("neg_grant_refused_register_after_cutoff", code != 200 and not j.get("granted"),
             R["negatives"]["grant_refused_register_after_cutoff"])
        res = tx("stranger", REGISTER_GAS, "wasm", "execute", contract, json.dumps({"register_notes": sbody}))
        neg("register_after_cutoff", res, "registration closed")
        try: smart(contract, {"check_register_notes": sbody}); r_ = "query ok"
        except Exception as e: r_ = str(e)[-200:]
        step("neg_check_register_notes_after_cutoff", "registration closed" in r_, r_)
        # sweep burns: contract -> 0, owner pays only the fee, total uterp supply drops by the balance
        c0, o0 = bal(contract), bal(OWNER)
        if GOV_ROLES:
            res, pid, st = gov_submit("sweep", [{"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": GOV, "contract": contract,
                                                 "msg": {"sweep_unclaimed": {}}, "funds": []}], "proposer")
            step("sweep_via_gov_burns", passed(st) and bal(contract) == 0, "proposal %s %s contract %d -> %d" % (pid, st and st["status"], c0, bal(contract)))
            return
        res = rec("sweep", tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"sweep_unclaimed": {}})))
        fee = int(3000000 * GP + 0.999999); hs = int(res.get("height", 0) or 0)
        s_before, s_after, minted = supply(hs - 1), supply(hs), minted_at(hs)
        burned = s_before + minted - s_after
        R["sweep_burn"] = {"height": hs, "contract_before": c0, "supply_before": s_before, "supply_after": s_after,
                           "minted_in_block": minted, "burned": burned, "mode": attr(res, "mode"), "amount_attr": attr(res, "amount")}
        burn_ev = [e for e in res.get("events", []) if e.get("type") == "burn"]
        step("sweep_unclaimed_burns", res.get("code") == 0 and bal(contract) == 0 and o0 - bal(OWNER) == fee and attr(res, "mode") == "burn"
             and bool(burn_ev), "burned %d contract now %d owner -%d (fee %d) %s" % (c0, bal(contract), o0 - bal(OWNER), fee, res.get("raw_log", "")[:120]))
        step("sweep_supply_drops_by_balance", burned == c0,
             "supply %d -> %d (+%d minted in block %d) burned %d == contract balance %d" % (s_before, s_after, minted, hs, burned, c0))

if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        import traceback; traceback.print_exc(); step("harness_exception", False, repr(e)[:300])
    finally:
        save()
        p = sum(s["pass"] for s in R["steps"]); f = len(R["steps"]) - p
        log("E2E_RESULT passed=%d failed=%d report=%s" % (p, f, os.path.join(RUN, "report.json")))
        sys.exit(1 if f else 0)
