#!/usr/bin/env python3
"""Headstash v0.0.1 dry run, Foundation-DAO roles (loopback only, NO_LANE=1, real vk_id 3 proofs).

Mirrors the mainnet sequence agreed for the initial headstash:
  0. local Foundation DAO from the exact morocco-1 DAO DAO bytes (core 23, proposal-single 24, pre-propose 31,
     voting-cw4 56, cw4-group 61; sha256 == on-chain data_hash), same module config (majority, 20% quorum,
     115200 s, close_on_failure, members-only pre-propose, no deposit), 3 members x 1000.
  1. plain MsgStoreCode of cw-headstash (+ frontend when FE_WASM is set) from a normal account
     (no x/gov, no DAO), instantiate permission AnyOfAddresses [DAO, x/gov].
  2. DAO proposal 1 (member proposes via pre-propose, 2/3 vote yes, execute): per token T in (terpy, thioly):
     MsgCreateDenom(T), MsgSetDenomMetadata, MsgMint(total allocation -> DAO), Instantiate2 headstash-<T>
     (admin = owner = DAO, 1 unit), SetEpochCampaign (per-instance ctx, derive_nd(factory/<DAO>/T)),
     bank send (allocation - 1). Supply of each token == allocation, DAO left with 0.
  3. x/gov proposal: MsgStoreFullCircuit only (vk_id 3, sender gov; circuit upload is Nobody, as on mainnet).
  4. DAO proposal 2: bind_note_circuit on both instances.
  5. claims on both instances (relay fee grants, real proofs), then DAO emergency reclaim (thioly, before
     cutoff -> community pool) and DAO sweep after cutoff (terpy, burn; supply drops by the escrow).
Env: as run_direct.py (TERPD, WASM, NOTE_EX, ELIG_BIN, WAVS_DISABLED=1, HEADSTASH_HOOK=0), DAO_WASM_DIR, FE_WASM.
"""
import base64, hashlib, json, os, subprocess, sys, time
os.environ.setdefault("TOKEN", "existing"); os.environ.setdefault("DENOM_RAW", "uterp")
import run_direct as d
import run as b
from run import R, log, step, save, b64, h2b64, terpd, q, height, tx, attr, smart, gas_used, HOME, CHAIN_ID, REST, RUN, \
    RELAY_BIN, RELAY_BIND, ELIG_BIN, DENOMS, NOTE_EX, NOTE_VK_HASH, TERPD

DAO_WASM = os.environ.get("DAO_WASM_DIR", "/tmp/hsdao/wasm")
DAO_SHA = {"core": "9cac5bcc", "single": "601d91b5", "prepropose": "2c87d60c", "voting": "84e520f8", "group": "3b194aac"}
FE_WASM = os.environ.get("FE_WASM", "")
GOV = d.GOV
TOKENS = ("terpy", "thioly")
NULL_ADMIN = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw"  # bech32 of 20 zero bytes; empty new_admin fails ValidateBasic
d.SWEEP = False  # flow(): no owner-key sweep/reclaim; the DAO does those below

# ------------------------------------------------------------------ protobuf (tokenfactory, bank metadata)
def _v(n):
    o = bytearray()
    while True:
        b_ = n & 0x7F; n >>= 7
        if n: o.append(b_ | 0x80)
        else: o.append(b_); return bytes(o)
def _s(f, s):
    s = s.encode() if isinstance(s, str) else s
    return _v((f << 3) | 2) + _v(len(s)) + s
def _u(f, n): return _v(f << 3) + _v(n)
def coin(denom, amt): return _s(1, denom) + _s(2, str(amt))
def any_msg(url, raw): return {"stargate": {"type_url": url, "value": base64.b64encode(raw).decode()}}
def tf_create(sender, sub): return any_msg("/osmosis.tokenfactory.v1beta1.MsgCreateDenom", _s(1, sender) + _s(2, sub))
def tf_mint(sender, denom, amt, to): return any_msg("/osmosis.tokenfactory.v1beta1.MsgMint", _s(1, sender) + _s(2, coin(denom, amt)) + _s(3, to))
def tf_metadata(sender, denom, sub):
    units = _s(2, _s(1, denom) + _u(2, 0)) + _s(2, _s(1, sub) + _u(2, 6))
    md = _s(1, "%s: headstash v0.0.1 dry-run token (Terp Foundation)" % sub) + units + _s(3, denom) + _s(4, sub) + _s(5, sub) + _s(6, sub.upper())
    return any_msg("/osmosis.tokenfactory.v1beta1.MsgSetDenomMetadata", _s(1, sender) + _s(2, md))
def tf_change_admin(sender, denom, new): return any_msg("/osmosis.tokenfactory.v1beta1.MsgChangeAdmin", _s(1, sender) + _s(2, denom) + _s(3, new))
def wasm_exec(contract, msg): return {"wasm": {"execute": {"contract_addr": contract, "msg": b64(json.dumps(msg).encode()), "funds": []}}}

# ------------------------------------------------------------------ local Foundation DAO
def store(frm, path, gas, anyof=None):
    a = ["wasm", "store", path] + (["--instantiate-anyof-addresses", ",".join(anyof)] if anyof else [])
    res = tx(frm, gas, *a)
    return res, attr(res, "code_id")

def setup_dao(members):
    ids = {}
    for n in ("core", "single", "prepropose", "voting", "group"):
        p = os.path.join(DAO_WASM, n + ".wasm"); sha = hashlib.sha256(open(p, "rb").read()).hexdigest()
        res, cid = store("owner", p, 60000000)
        ids[n] = cid
        step("dao_code_%s_mainnet_bytes" % n, res.get("code") == 0 and sha.startswith(DAO_SHA[n]), "code %s sha %s" % (cid, sha[:16]))
    mod = lambda cid, msg, label: {"code_id": int(cid), "msg": b64(json.dumps(msg).encode()), "admin": {"core_module": {}}, "funds": [], "label": label}
    pre = {"deposit_info": None, "submission_policy": {"specific": {"dao_members": True, "allowlist": [], "denylist": []}}, "extension": {}}
    single = {"threshold": {"threshold_quorum": {"threshold": {"majority": {}}, "quorum": {"percent": "0.2"}}},
              "max_voting_period": {"time": 115200}, "min_voting_period": None, "only_members_execute": False, "allow_revoting": False,
              "pre_propose_info": {"module_may_propose": {"info": mod(ids["prepropose"], pre, "pre-propose-A")}},
              "close_proposal_on_execution_failure": True, "veto": None}
    voting = {"group_contract": {"new": {"cw4_group_code_id": int(ids["group"]), "initial_members": [{"addr": a, "weight": 1000} for a in members]}}}
    core = {"admin": None, "name": "TerpNet Foundation (local mirror)", "description": "local mirror of terp14w2qva6...", "image_url": None,
            "automatically_add_cw20s": True, "automatically_add_cw721s": True,
            "voting_module_instantiate_info": mod(ids["voting"], voting, "voting"),
            "proposal_modules_instantiate_info": [mod(ids["single"], single, "proposal-A")], "initial_items": None, "dao_uri": None}
    res = tx("owner", 8000000, "wasm", "instantiate", ids["core"], json.dumps(core), "--label", "foundation-local", "--no-admin")
    dao = attr(res, "_contract_address")
    step("dao_instantiated", res.get("code") == 0 and bool(dao), "%s %s" % (dao, res.get("raw_log", "")[:300]))
    if not dao: return None
    pm = smart(dao, {"proposal_modules": {}})[0]["address"]
    pp = smart(pm, {"proposal_creation_policy": {}})["module"]["addr"]
    cfg = smart(pm, {"config": {}}); pcfg = smart(pp, {"config": {}})
    step("dao_config_mirrors_mainnet", cfg["threshold"] == single["threshold"] and cfg["max_voting_period"] == {"time": 115200}
         and cfg["close_proposal_on_execution_failure"] is True and pcfg["deposit_info"] is None
         and pcfg["submission_policy"] == pre["submission_policy"], json.dumps({"single": cfg, "pre": pcfg})[:300])
    return {"core": dao, "proposal": pm, "prepropose": pp, "codes": ids}

def dao_run(D, name, title, msgs, proposer="owner", voters=("owner", "dao_m2")):
    """Member proposes through the pre-propose module, members vote yes, anyone executes."""
    body = {"propose": {"msg": {"propose": {"title": title, "description": "local dry run: " + name, "msgs": msgs}}}}
    json.dump(body, open(os.path.join(RUN, "dao_%s.json" % name), "w"), indent=1)
    res = tx(proposer, 6000000, "wasm", "execute", D["prepropose"], json.dumps(body))
    pid = attr(res, "proposal_id")
    if res.get("code") != 0 or not pid: return res, None, None
    pid = int(pid)
    for v in voters:
        rv = tx(v, 1500000, "wasm", "execute", D["proposal"], json.dumps({"vote": {"proposal_id": pid, "vote": "yes"}}))
        if rv.get("code") != 0: log("dao vote %s failed %s" % (v, rv.get("raw_log", "")[:200]))
    st0 = smart(D["proposal"], {"proposal": {"proposal_id": pid}})["proposal"]["status"]
    ex = tx("stranger", 60000000, "wasm", "execute", D["proposal"], json.dumps({"execute": {"proposal_id": pid}}))
    st = smart(D["proposal"], {"proposal": {"proposal_id": pid}})["proposal"]["status"]
    R.setdefault("dao_proposals", {})[name] = {"id": pid, "status_before_execute": st0, "status": st, "exec_gas": gas_used(ex),
                                                "exec_code": ex.get("code"), "msgs": len(msgs)}
    return ex, pid, st

def tbal_of(addr, denom):
    for c in q("bank", "balances", addr).get("balances", []):
        if c["denom"] == denom: return int(c["amount"])
    return 0

def prefixed(prefix):
    def s(name, ok, detail=""): return b.step(prefix + name, ok, detail)
    return s

def start_relay(contract):
    env = dict(os.environ, HEADSTASH_RELAY_HOME=HOME, HEADSTASH_RELAY_FROM="relayer", HEADSTASH_KEYRING_BACKEND="test",
               HEADSTASH_REST=REST, HEADSTASH_CONTRACT=contract, HEADSTASH_CHAIN_ID=CHAIN_ID, HEADSTASH_RELAY_BIND=RELAY_BIND,
               HEADSTASH_GAS_PRICE="0.025uterp", HEADSTASH_CLAIM_GAS=str(d.CLAIM_GAS), HEADSTASH_REGISTER_GAS=str(d.REGISTER_GAS),
               TERPD=TERPD, PATH="/usr/local/bin:" + os.environ.get("PATH", ""))
    env.pop("HEADSTASH_ELIGIBILITY", None)
    lp = os.path.join(RUN, "relay-%s.log" % contract[-6:])
    relay = subprocess.Popen([RELAY_BIN], env=env, stdout=open(lp, "w"), stderr=subprocess.STDOUT)
    for _ in range(60):
        time.sleep(1)
        if "relay_up" in open(lp).read(): return relay, True
        if relay.poll() is not None: break
    return relay, False

KMS_BIN = os.environ.get("KMS_DEPLOY_BIN", "")  # mainnet_deploy: prop 1 goes through generic dao-propose / vote / dao-execute
KENV = {}

def kms(who, phase, D, name, *extra):
    a = [KMS_BIN, phase, "--signer", "local-test", "--broadcast", "--network", CHAIN_ID, "--chain-id", CHAIN_ID,
         "--grpc", "http://127.0.0.1:49091", "--rpc", "http://127.0.0.1:46667", "--lcd", REST, "--gas-denom", "uterp", "--gas-price", "0.025",
         "--admin", D["core"], "--state", os.path.join(RUN, "kms-state-%s-%s-%s.json" % (name, who, phase)), *extra]
    env = dict(os.environ, TERP_SCRIPT_BROADCAST="1", TERP_SCRIPT_BROADCAST_CHAIN=CHAIN_ID, LOCAL_MNEMONIC=KENV[who])
    p = subprocess.run(a, env=env, capture_output=True, text=True)
    open(os.path.join(RUN, "kms_%s_%s_%s.log" % (name, who, phase)), "w").write(p.stdout + p.stderr)
    return p

def kms_dao_run(D, name, title, msgs):
    """dao_m3: generic dao-propose --dao (sha-pinned file) --vote yes; dao_m2: `vote` alias, then dao-execute."""
    import re
    body = {"propose": {"msg": {"propose": {"title": title, "description": "local dry run: " + name, "msgs": msgs}}}}
    f = os.path.join(RUN, "dao_%s.propose.json" % name); json.dump(body, open(f, "w"), indent=1)
    sha = hashlib.sha256(open(f, "rb").read()).hexdigest()
    bad = kms("dao_m3", "dao-propose", D, name + "-bad", "--dao", D["core"], "--propose-file", f, "--propose-sha", "00" * 32, "--vote", "yes")
    step(name + "_kms_sha_mismatch_refused_before_grant", bad.returncode != 0 and "refused before any grant" in bad.stdout + bad.stderr
         and "dao-propose:" not in bad.stdout, (bad.stdout + bad.stderr)[-200:])
    p = kms("dao_m3", "dao-propose", D, name, "--dao", D["core"], "--propose-file", f, "--propose-sha", sha, "--vote", "yes")
    m = re.search(r"proposal (\d+)", p.stdout)
    step(name + "_kms_dao_propose_resolved_and_voted", p.returncode == 0 and bool(m) and "voted yes" in p.stdout and D["prepropose"] in p.stdout
         and "instantiate2 check: ok (2 addresses" in p.stdout, (p.stdout + p.stderr)[-300:])
    if not m: return {}, None, None
    pid = int(m.group(1))
    st0 = smart(D["proposal"], {"proposal": {"proposal_id": pid}})["proposal"]["status"]
    v = kms("dao_m2", "vote", D, name, "--foundation-pre-propose", D["prepropose"], "--proposals", str(pid))
    st1 = smart(D["proposal"], {"proposal": {"proposal_id": pid}})["proposal"]["status"]
    step(name + "_legacy_vote_alias", v.returncode == 0 and st0 == "open" and st1 == "passed", "before %s after %s %s" % (st0, st1, (v.stdout + v.stderr)[-200:]))
    ex = kms("dao_m2", "dao-execute", D, name, "--dao", D["core"], "--proposals", str(pid))
    st = smart(D["proposal"], {"proposal": {"proposal_id": pid}})["proposal"]["status"]
    step(name + "_kms_dao_execute_second_member", ex.returncode == 0 and "executed" in ex.stdout and st == "executed", (ex.stdout + ex.stderr)[-300:])
    R.setdefault("dao_proposals", {})[name] = {"id": pid, "via": "mainnet_deploy dao-propose/vote/dao-execute", "sha": sha, "status": st, "msgs": len(msgs)}
    return {"code": 0 if ex.returncode == 0 else 1, "raw_log": ex.stdout[-300:]}, pid, st

def main():
    b.R = R
    addrs = dict(l.strip().split("=", 1) for l in open(os.path.join(HOME, "addresses.env")) if "=" in l)
    OWNER = addrs["OWNER_ADDR"]; PROPOSER = addrs["PROPOSER_ADDR"]
    RELAYER = terpd("keys", "show", "relayer", "-a", "--keyring-backend", "test", "--home", HOME).strip()
    R["mode"] = "dao-roles"; R["addresses"] = dict(addrs, RELAYER_ADDR=RELAYER)
    R["chain"] = {"chain_id": CHAIN_ID, "start_height": height()}
    wp = q("wasm", "params"); tp = q("tokenfactory", "params")
    step("genesis_mirrors_mainnet_params", wp["params" if "params" in wp else "code_upload_access"] is not None
         and (wp.get("params", wp))["circuit_upload_access"]["permission"] == "Nobody"
         and (wp.get("params", wp))["code_upload_access"]["permission"] == "Everybody"
         and (tp.get("params", tp))["denom_creation_fee"] == [], json.dumps({"wasm": wp, "tf": tp})[:300])

    # identities, notes, circuit blob (same as run_direct)
    import secrets
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives import serialization
    evm_priv = secrets.randbelow(b.N - 1) + 1
    pt = b.pmul(evm_priv); evm_addr = b.kec(pt[0].to_bytes(32, "big") + pt[1].to_bytes(32, "big"))[12:]
    sol_sk = Ed25519PrivateKey.generate()
    sol_pk = sol_sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    ids = {"evm": (0x01, evm_addr, evm_addr.hex()), "sol": (0x02, sol_pk, "sol:" + sol_pk.hex())}
    comm = os.path.join(RUN, "communities", "mvk"); os.makedirs(comm, exist_ok=True)
    open(os.path.join(comm, "mvk.csv"), "w").write("addr,amount\n0x%s,200008\n%s,3.5\n" % (evm_addr.hex(), b.b58(sol_pk)))
    elig = os.path.join(RUN, "eligibility")
    subprocess.run([ELIG_BIN, os.path.join(RUN, "communities"), elig, "--decimals", "mvk=6", "--denominations"], check=True, capture_output=True)
    ntree = json.load(open(os.path.join(elig, "notes", "tree.json")))
    paths = [json.loads(l) for l in open(os.path.join(elig, "notes", "note_paths.jsonl"))]
    notes = {k: sorted([p for p in paths if p["identity"] == ids[k][2]], key=lambda p: p["note_index"]) for k in ids}
    step("note_eligibility_tree", ntree["depth"] == 17 and {k: len(v) for k, v in notes.items()} == {"evm": 43, "sol": 2},
         "root %s notes %s" % (ntree["root_hex"], ntree["notes"]))
    classes = {}
    for k in ids:
        for p in notes[k]: classes[p["class"]] = classes.get(p["class"], 0) + 1
    amounts = [{"amount": DENOMS[c], "cap": n} for c, n in sorted(classes.items())]
    ALLOC = sum(int(a["amount"]) * a["cap"] for a in amounts)
    R["allocation_total"] = ALLOC
    cdir = os.path.join(RUN, "circuit"); os.makedirs(cdir, exist_ok=True)
    subprocess.run([NOTE_EX + "/bench_note_proofs", cdir, "0"], check=True, capture_output=True)
    blob = open(os.path.join(cdir, "blob.bin"), "rb").read()
    step("circuit_blob_hash", hashlib.sha256(blob).hexdigest() == NOTE_VK_HASH, "sha %s" % hashlib.sha256(blob).hexdigest())

    # 0. local Foundation DAO
    if KMS_BIN:  # dao_m2/dao_m3 are mainnet_deploy local-test signers; throwaway phrases stay in this process (never printed or saved)
        def kms_wallet(n):
            terpd("keys", "delete", n, "-y", "--keyring-backend", "test", "--home", HOME, check=False)
            KENV[n] = json.loads(terpd("keys", "add", n, "--keyring-backend", "test", "--home", HOME, "--output", "json"))["mnemonic"]
            return terpd("keys", "show", n, "-a", "--keyring-backend", "test", "--home", HOME).strip()
        m2, m3 = kms_wallet("dao_m2"), kms_wallet("dao_m3")
    else:
        m2, m3 = d.new_wallet("dao_m2"), d.new_wallet("dao_m3")
    tx("owner", 300000, "bank", "send", OWNER, m2, "50000000uterp"); tx("owner", 300000, "bank", "send", OWNER, m3, "50000000uterp")
    D = setup_dao([OWNER, m2, m3])
    if not D: return
    DAO = D["core"]; R["dao"] = D
    neg_body = {"propose": {"msg": {"propose": {"title": "x", "description": "x", "msgs": []}}}}
    d.neg("dao_non_member_cannot_propose", tx("stranger", 3000000, "wasm", "execute", D["prepropose"], json.dumps(neg_body)))

    # 1. plain uploads (normal account, no gov / DAO)
    whs = open(d.WASM, "rb").read(); hs_sha = hashlib.sha256(whs).hexdigest()
    res, code_id = store("proposer", d.WASM, 250000000)  # Everybody, as cw-orch-kms stores
    ci = q("wasm", "code-info", code_id) if code_id else {}
    step("plain_store_cw_headstash", res.get("code") == 0 and (ci.get("checksum") or ci.get("data_hash") or "").lower() == hs_sha
         and ci.get("creator") == PROPOSER and ci.get("instantiate_permission", {}).get("permission") == "Everybody",
         "code %s sha %s creator %s perm %s gas %d" % (code_id, hs_sha[:16], ci.get("creator"), json.dumps(ci.get("instantiate_permission")), gas_used(res)))
    R["code_id"] = code_id; R["wasm_sha256"] = hs_sha
    if FE_WASM and os.path.exists(FE_WASM):
        fsha = hashlib.sha256(open(FE_WASM, "rb").read()).hexdigest()
        res, fcid = store("proposer", FE_WASM, 120000000)
        fci = q("wasm", "code-info", fcid) if fcid else {}
        step("plain_store_frontend", res.get("code") == 0 and (fci.get("checksum") or fci.get("data_hash") or "").lower() == fsha,
             "code %s sha %s gas %d" % (fcid, fsha[:16], gas_used(res)))
        R["fe_code_id"] = fcid; R["fe_sha256"] = fsha
    zb = terpd("debug", "addr", "0000000000000000000000000000000000000000")
    step("null_admin_is_20_zero_bytes", NULL_ADMIN in zb, zb.strip().replace("\n", " ")[:200])

    # 2. DAO proposal 1: create denoms, metadata, mint allocation, instantiate2, campaign, fund
    cut = {"thioly": height() + d.CUT_AHEAD, "terpy": height() + 2 * d.CUT_AHEAD}
    chk = (ci.get("checksum") or ci.get("data_hash")).lower()
    inst, msgs = {}, []
    for t in TOKENS:
        denom = "factory/%s/%s" % (DAO, t); label = "headstash-2026-1-%s" % t
        salt = hashlib.sha256(("headstash:" + label).encode()).hexdigest()
        addr = terpd("q", "wasm", "build-address", chk, DAO, salt).strip().split()[-1]
        nd = d.derive_nd(denom); ctx = d.direct_ctx(CHAIN_ID, addr)
        init = {"genesis_root": b64(bytes(32)), "genesis_label": label, "token_strategy": {"existing_fungible": {"proof": h2b64(nd), "raw": denom}},
                "wavs": None, "frontend_contract": None, "pauser": DAO}
        camp = {"set_epoch_campaign": {"campaign": {
            "ctx": b64(ctx), "backend": "halo2", "cutoff_height": cut[t], "bech32_hrp": "terp", "denom": h2b64(nd),
            "amounts": amounts, "reserved": [], "trusted": [b64(bytes(32))], "trusted_verify_threshold": 1, "trusted_sign_threshold": 1,
            "max_rows": 8, "allocation_root": b64(bytes(32)), "root_id": 0, "vk_id": 3, "vk_hash": h2b64(NOTE_VK_HASH), "sweep_recipient": "",
            "registration": {"eligibility_root": h2b64(ntree["root_hex"]), "eligibility_depth": 17, "min_anonymity_set": 2,
                             "anchor_start_height": 1, "anchor_interval": 1}}}}
        inst[t] = {"addr": addr, "denom": denom, "nd": nd, "ctx": ctx, "label": label, "salt": salt, "cut": cut[t], "init": init}
        msgs += [tf_create(DAO, t), tf_metadata(DAO, denom, t), tf_mint(DAO, denom, ALLOC, DAO),
                 {"wasm": {"instantiate2": {"admin": DAO, "code_id": int(code_id), "label": label, "msg": b64(json.dumps(init).encode()),
                                            "funds": [{"denom": denom, "amount": "1"}], "salt": b64(bytes.fromhex(salt))}}},
                 wasm_exec(addr, camp),
                 {"bank": {"send": {"to_address": addr, "amount": [{"denom": denom, "amount": str(ALLOC - 1)}]}}}]
    msgs += [tf_change_admin(DAO, inst[t]["denom"], NULL_ADMIN) for t in TOKENS]  # renounce last
    R["instances"] = {t: {k: (v.hex() if isinstance(v, bytes) else v) for k, v in i.items()} for t, i in inst.items()}
    ex, pid, st = (kms_dao_run if KMS_BIN else dao_run)(D, "prop1_deploy", "Headstash v0.0.1 dry run: terpy + thioly", msgs)
    step("dao_prop1_executed", st == "executed", "proposal %s %s gas %d %s" % (pid, st, gas_used(ex), ex.get("raw_log", "")[:300]))
    if st != "executed": return
    for t, i in inst.items():
        c = i["addr"]; cinfo = q("wasm", "contract", c); cinfo = cinfo.get("contract_info", cinfo)
        own = smart(c, {"ownership": {}}).get("owner")
        step("%s_instantiated_dao_admin_owner" % t, cinfo.get("admin") == DAO and cinfo.get("creator") == DAO and own == DAO and cinfo.get("label") == i["label"],
             "%s admin %s creator %s owner %s" % (c, cinfo.get("admin"), cinfo.get("creator"), own))
        sup = d.supply(height(), i["denom"])
        step("%s_supply_equals_allocation" % t, sup == ALLOC and tbal_of(c, i["denom"]) == ALLOC and tbal_of(DAO, i["denom"]) == 0,
             "supply %d alloc %d contract %d dao %d" % (sup, ALLOC, tbal_of(c, i["denom"]), tbal_of(DAO, i["denom"])))
        md = q("bank", "denom-metadata", i["denom"]); md = md.get("metadata", md)
        step("%s_denom_metadata" % t, md.get("display") == t and md.get("symbol") == t.upper() and any(u.get("exponent") == 6 for u in md.get("denom_units", [])), json.dumps(md)[:200])
        ta = q("tokenfactory", "denom-authority-metadata", i["denom"]); adm = ta.get("authority_metadata", ta).get("admin")
        step("%s_tf_admin_renounced" % t, adm == NULL_ADMIN, json.dumps(ta)[:200])
        info = smart(c, {"direct_claim": {}})
        step("%s_direct_ctx_matches" % t, base64.b64decode(info["expected_ctx"]) == i["ctx"], json.dumps(info)[:160])
        d.neg("%s_bind_before_circuit_refused" % t, tx("owner", 3000000, "wasm", "execute", c, json.dumps({"bind_note_circuit": {"zk_id": 1}})))
    if os.environ.get("STOP_AFTER_PROP1") == "1": return
    # tokenfactory is locked: the DAO itself can no longer mint, set metadata or change admin
    for t in TOKENS:
        i = inst[t]; s0 = d.supply(height(), i["denom"])
        for nm, m in (("mint", tf_mint(DAO, i["denom"], 1, DAO)), ("setmetadata", tf_metadata(DAO, i["denom"], t + "x")),
                      ("changeadmin", tf_change_admin(DAO, i["denom"], DAO))):
            ex, pid, st = dao_run(D, "neg_%s_%s" % (t, nm), "%s %s after renounce (must fail)" % (t, nm), [m])
            step("%s_dao_%s_after_renounce_fails" % (t, nm), st == "execution_failed", "proposal %s %s" % (pid, st))
        ta = q("tokenfactory", "denom-authority-metadata", i["denom"]); md = q("bank", "denom-metadata", i["denom"]); md = md.get("metadata", md)
        step("%s_locked_state_unchanged" % t, ta.get("authority_metadata", ta).get("admin") == NULL_ADMIN and d.supply(height(), i["denom"]) == s0 == ALLOC
             and md.get("display") == t, "admin %s supply %d" % (ta.get("authority_metadata", ta).get("admin"), d.supply(height(), i["denom"])))
    # instantiate permission is Everybody: a stranger instance (same code, label, salt) is separate and leaves the DAO instances alone
    i = inst["terpy"]; snap = {t: (smart(inst[t]["addr"], {"ownership": {}}).get("owner"), tbal_of(inst[t]["addr"], inst[t]["denom"]),
                                   json.dumps(smart(inst[t]["addr"], {"direct_claim": {}}), sort_keys=True)) for t in TOKENS}
    sinit = dict(i["init"], token_strategy={"existing_fungible": {"proof": h2b64(d.derive_nd("uterp")), "raw": "uterp"}}, pauser=None)
    rs = tx("stranger", 8000000, "wasm", "instantiate2", code_id, json.dumps(sinit), i["salt"], "--hex", "--label", i["label"], "--no-admin", "--amount", "1uterp")
    sa = attr(rs, "_contract_address")
    sci = q("wasm", "contract", sa) if sa else {}; sci = sci.get("contract_info", sci)
    after = {t: (smart(inst[t]["addr"], {"ownership": {}}).get("owner"), tbal_of(inst[t]["addr"], inst[t]["denom"]),
                 json.dumps(smart(inst[t]["addr"], {"direct_claim": {}}), sort_keys=True)) for t in TOKENS}
    step("stranger_instance_isolated_from_dao_instances", rs.get("code") == 0 and sa and sa not in (inst["terpy"]["addr"], inst["thioly"]["addr"])
         and sci.get("creator") != DAO and after == snap and all(after[t][0] == DAO and after[t][1] == ALLOC for t in TOKENS),
         "stranger %s creator %s code %s %s" % (sa, sci.get("creator"), rs.get("code"), rs.get("raw_log", "")[:200]))
    d.neg("dao_member_key_is_not_owner_reclaim", tx("owner", 3000000, "wasm", "execute", inst["thioly"]["addr"], json.dumps({"emergency_reclaim": {}})))

    # 3. x/gov: circuit only (direct store refused: circuit_upload_access Nobody)
    d.neg("direct_circuit_store_refused_nobody", tx("stranger", 60000000, "wasm", "store-full-circuit", os.path.join(cdir, "params.bin"),
          os.path.join(cdir, "vk_body.bin"), "--k", "11", "--circuit-type", "0", "--curve-type", "0"))
    d.open_session(PROPOSER, "eph_c")
    res_c = d.circuit_via_gov(PROPOSER, cdir)
    d.close_session(PROPOSER)
    zk_id = None
    for ev in res_c.get("events", []):
        for at in ev.get("attributes", []):
            if at["key"] in ("zk_id", "circuit_id") and at["value"].isdigit() and "circuit" in ev["type"]: zk_id = int(at["value"])
    if zk_id is None:  # the store event is in the end-block of the proposal; the first circuit on a fresh chain is 1
        zk_id = int(os.environ.get("ZK_ID", "1"))
    R["zk_id"] = zk_id

    # 4. DAO proposal 2: bind both
    ex, pid, st = dao_run(D, "prop2_bind", "Headstash v0.0.1 dry run: bind vk_id 3 circuit",
                          [wasm_exec(inst[t]["addr"], {"bind_note_circuit": {"zk_id": zk_id}}) for t in TOKENS])
    step("dao_prop2_bind_executed", st == "executed", "proposal %s %s zk_id %s gas %d %s" % (pid, st, zk_id, gas_used(ex), ex.get("raw_log", "")[:200]))
    if st != "executed": return
    ex, pid, st = dao_run(D, "prop2b_rebind", "rebind (must fail)", [wasm_exec(inst["terpy"]["addr"], {"bind_note_circuit": {"zk_id": zk_id}})])
    step("dao_rebind_execution_failed", st == "execution_failed", "proposal %s %s (circuit is fixed)" % (pid, st))
    save()

    # 5. claims on both instances: thioly (then DAO reclaim before cutoff), terpy (then cutoff + DAO sweep burn)
    R.setdefault("gas", {}); R["hook"] = {"enabled": False}
    for t in ("thioly", "terpy"):
        i = inst[t]; c = i["addr"]
        R["gas"] = {}
        d.TD, d.DENOM, d.FUND_AMT = i["denom"], i["nd"], ALLOC
        d.step = prefixed(t + ":"); d.REWARD.clear()
        for k in ("evm", "sol"): d.REWARD[k] = d.new_wallet("reward_%s_%s" % (t, k))
        relay, up = start_relay(c)
        try:
            b.step(t + ":relay_up", up, "")
            if not up: return
            d.flow(c, i["ctx"], ids, notes, evm_priv, sol_sk, sol_pk, evm_addr, RELAYER, OWNER, i["cut"])
        finally:
            d.step = b.step
            relay.terminate()
            try: relay.wait(10)
            except Exception: relay.kill()
        claimed = sum(int(x["amount"]) for x in R["claims"]); R.setdefault("claims_by_token", {})[t] = list(R["claims"])
        R.setdefault("gas_by_token", {})[t] = dict(R["gas"])
        step("%s_claims_paid_in_%s" % (t, t), claimed > 0 and all(tbal_of(x["wallet"], i["denom"]) >= int(x["amount"]) for x in R["claims"])
             and tbal_of(c, i["denom"]) == ALLOC - claimed, "%d claims %d paid, escrow %d" % (len(R["claims"]), claimed, tbal_of(c, i["denom"])))
        if t == "thioly":
            d.neg("thioly_sweep_before_cutoff_refused", tx("owner", 3000000, "wasm", "execute", c, json.dumps({"sweep_unclaimed": {}})))
            c0, p0, s0 = tbal_of(c, i["denom"]), d.community_pool(i["denom"]), d.supply(height(), i["denom"])
            ex, pid, st = dao_run(D, "prop3_reclaim_thioly", "emergency reclaim thioly", [wasm_exec(c, {"emergency_reclaim": {}})])
            p1 = d.community_pool(i["denom"])
            step("thioly_dao_reclaim_before_cutoff", st == "executed" and height() <= i["cut"] and tbal_of(c, i["denom"]) == 0 and p1 - p0 == c0
                 and d.supply(height(), i["denom"]) == s0, "proposal %s %s escrow %d -> 0, pool +%d, supply %d" % (pid, st, c0, p1 - p0, s0))
            ex, pid, st = dao_run(D, "prop3b_reclaim_again", "reclaim again (must fail)", [wasm_exec(c, {"emergency_reclaim": {}})])
            step("thioly_second_reclaim_execution_failed", st == "execution_failed", "proposal %s %s" % (pid, st))
        else:
            log("waiting for cutoff", i["cut"], "now", height())
            while height() <= i["cut"] + 1: time.sleep(2)
            ex, pid, st = dao_run(D, "prop4_reclaim_late", "reclaim after cutoff (must fail)", [wasm_exec(c, {"emergency_reclaim": {}})])
            step("terpy_reclaim_after_cutoff_execution_failed", st == "execution_failed", "proposal %s %s" % (pid, st))
            c0 = tbal_of(c, i["denom"])
            ex, pid, st = dao_run(D, "prop5_sweep_terpy", "sweep (burn) terpy after cutoff", [wasm_exec(c, {"sweep_unclaimed": {}})])
            hs = int(ex.get("height", 0) or 0)
            s_before, s_after = d.supply(hs - 1, i["denom"]), d.supply(hs, i["denom"])
            step("terpy_dao_sweep_burns_after_cutoff", st == "executed" and tbal_of(c, i["denom"]) == 0 and s_before - s_after == c0,
                 "proposal %s %s escrow %d burned %d supply %d -> %d" % (pid, st, c0, s_before - s_after, s_before, s_after))
            claimed = sum(int(x["amount"]) for x in R["claims_by_token"]["terpy"])
            step("terpy_final_supply_equals_claimed", s_after == claimed, "supply %d claimed %d" % (s_after, claimed))
        save()

if __name__ == "__main__":
    try: main()
    except Exception as e:
        import traceback; traceback.print_exc(); step("harness_exception", False, repr(e)[:300])
    finally:
        save()
        p = sum(s["pass"] for s in R["steps"]); f = len(R["steps"]) - p
        log("E2E_RESULT passed=%d failed=%d report=%s" % (p, f, os.path.join(RUN, "report.json")))
        sys.exit(1 if f else 0)
