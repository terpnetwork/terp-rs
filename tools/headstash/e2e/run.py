#!/usr/bin/env python3
"""Headstash denomination-note (vk_id 3) local e2e harness (loopback only).

Needs a chain from bootstrap-chain.sh. Test identities (EVM key, Solana key,
membership secrets) live only in this process; nothing secret is written or
printed. Writes public results to $RUN_DIR/report.json.
"""
import base64, hashlib, json, os, secrets, subprocess, sys, time, signal
import requests
from Crypto.Hash import keccak
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.environ.get("CHAIN_HOME", "/tmp/headstash-mvk-chain")
CHAIN_ID = "headstash-mvk-e2e"
NODE = "tcp://127.0.0.1:46667"
RPC = "http://127.0.0.1:46667"
REST = "http://127.0.0.1:41327"
LANE = "http://127.0.0.1:19180"
RELAY_BIND = "127.0.0.1:18795"
TERPD = os.environ.get("TERPD", "/tmp/terpd-b2339a9c-build/terpd")
RELAY_BIN = os.environ.get("RELAY_BIN", "/tmp/hm-build-target/release/headstash-register-relay")
ELIG_BIN = os.environ.get("ELIG_BIN", "/tmp/hm-build-target/release/headstash-eligibility")
EX = os.environ.get("ZK_EX", "/tmp/hm-build-target/release/examples")
WAVS_BIN = os.environ.get("WAVS_BIN", "/tmp/cwhs-native-target/release/examples/print_wavs")
WASM = os.environ.get("WASM", "/tmp/hs-p2-opt-out/cw_headstash.wasm")
OPSIGN = os.path.join(HERE, "..", "..", "hash-market", "scripts", "headstash-operator-sign.py")
VK_HASH = "94d4326108e28451829b68d64df58cf975fafdb874a5a554f3cbe8931eefed11"
DENOM = "1d56a5879c78333d517942a36e7ea0932e7ffa1a1027e439279e8517580ea706"
CUT_AHEAD = int(os.environ.get("CUT_AHEAD", "220"))
RUN = os.environ.get("RUN_DIR") or os.path.join(HOME, "run-%d" % int(time.time()))
os.makedirs(RUN, exist_ok=True)
for a in (RELAY_BIND,):
    assert a.startswith("127.0.0.1:")
for u in (RPC, REST, LANE):
    assert u.startswith("http://127.0.0.1:")

R = {"steps": [], "tx": {}, "negatives": {}}
def log(*a): print(*a, flush=True)
def step(name, ok, detail=""):
    R["steps"].append({"name": name, "pass": bool(ok), "detail": detail}); log(("PASS " if ok else "FAIL ") + name, detail)
def save():
    json.dump(R, open(os.path.join(RUN, "report.json"), "w"), indent=2)
def b64(b): return base64.b64encode(b).decode()
def h2b64(h): return b64(bytes.fromhex(h))

# ---------------- secp256k1 (pure python, signing + recid) ----------------
P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
G = (0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798,
     0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8)
def padd(a, b):
    if a is None: return b
    if b is None: return a
    if a[0] == b[0] and (a[1] + b[1]) % P == 0: return None
    if a == b: l = 3 * a[0] * a[0] * pow(2 * a[1], -1, P) % P
    else: l = (b[1] - a[1]) * pow(b[0] - a[0], -1, P) % P
    x = (l * l - a[0] - b[0]) % P
    return (x, (l * (a[0] - x) - a[1]) % P)
def pmul(k, pt=G):
    r = None
    while k:
        if k & 1: r = padd(r, pt)
        pt = padd(pt, pt); k >>= 1
    return r
def evm_sign(priv, digest):
    z = int.from_bytes(digest, "big")
    while True:
        k = secrets.randbelow(N - 1) + 1
        Rp = pmul(k); r = Rp[0] % N
        if r == 0 or Rp[0] >= N: continue
        s = pow(k, -1, N) * (z + r * priv) % N
        if s == 0: continue
        rec = Rp[1] & 1
        if s > N // 2: s = N - s; rec ^= 1
        return r.to_bytes(32, "big") + s.to_bytes(32, "big") + bytes([27 + rec])
def kec(b): h = keccak.new(digest_bits=256); h.update(b); return h.digest()
def eip191(msg):
    m = msg.encode(); return kec(b"\x19Ethereum Signed Message:\n" + str(len(m)).encode() + m)

B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58(b):
    n = int.from_bytes(b, "big"); s = ""
    while n: n, r = divmod(n, 58); s = B58[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s

def pallas_rand():
    b = bytearray(secrets.token_bytes(32)); b[31] &= 0x3F; return bytes(b)

# ---------------- chain helpers ----------------
def terpd(*args, check=True):
    p = subprocess.run([TERPD, *args], capture_output=True, text=True)
    if check and p.returncode != 0:
        raise RuntimeError("terpd %s: %s" % (args[:3], p.stderr[-400:]))
    return p.stdout
def q(*args):
    return json.loads(terpd("q", *args, "--node", NODE, "-o", "json"))
def height():
    return int(requests.get(RPC + "/status", timeout=5).json()["result"]["sync_info"]["latest_block_height"])
def wait_tx(txhash, t=60):
    end = time.time() + t
    while time.time() < end:
        p = subprocess.run([TERPD, "q", "tx", txhash, "--node", NODE, "-o", "json"], capture_output=True, text=True)
        if p.returncode == 0 and p.stdout.strip():
            return json.loads(p.stdout)
        time.sleep(1)
    raise RuntimeError("tx not found " + txhash)
def tx(frm, gas, *args, amount=None):
    a = ["tx", *args, "--from", frm, "--home", HOME, "--keyring-backend", "test", "--chain-id", CHAIN_ID,
         "--node", NODE, "--gas", str(gas), "--gas-prices", "0.025uterp", "-b", "sync", "-y", "-o", "json"]
    if amount: a += ["--amount", amount]
    p = subprocess.run([TERPD, *a], capture_output=True, text=True)
    try: res = json.loads(p.stdout)
    except Exception: return {"code": -1, "raw_log": (p.stdout + p.stderr)[-600:]}
    if res.get("code", 0) != 0: return res
    return wait_tx(res["txhash"])
def attr(res, key):
    for ev in res.get("events", []):
        for at in ev.get("attributes", []):
            if at.get("key") == key: return at.get("value")
    return None
def smart(contract, msg):
    return q("wasm", "contract-state", "smart", contract, json.dumps(msg))["data"]
def bal(addr):
    for c in q("bank", "balances", addr).get("balances", []):
        if c["denom"] == "uterp": return int(c["amount"])
    return 0
def rec(name, res):
    R["tx"][name] = {"txhash": res.get("txhash"), "height": int(res.get("height", 0) or 0), "code": res.get("code")}
    save(); return res

def opsign_post(path, body_bytes):
    f = os.path.join(RUN, "body-%s.json" % path.strip("/").replace("/", "_"))
    open(f, "wb").write(body_bytes)
    key = os.path.join(HOME, "operator.key")
    pub = subprocess.run(["/usr/local/bin/python3", OPSIGN, "--key", key, "--pubkey-only"], capture_output=True, text=True, check=True).stdout.strip()
    sig = subprocess.run(["/usr/local/bin/python3", OPSIGN, "--key", key, "--method", "POST", "--path", path, "--body", f], capture_output=True, text=True, check=True).stdout.strip()
    return requests.post(LANE + path, data=body_bytes, timeout=600, headers={
        "content-type": "application/json", "x-hash-market-pubkey": pub, "x-hash-market-signature": sig})

NOTE_VK_HASH = "34d11437d957554f4008377dece5fac0f047e0e9cc012d15b3d09448a6cc5398"
DENOMS = [1_000_000, 2_000_000, 5_000_000, 10_000_000, 20_000_000, 50_000_000, 100_000_000,
          200_000_000, 500_000_000, 1_000_000_000, 2_000_000_000, 5_000_000_000]
NOTE_EX = os.environ.get("NOTE_EX", "/tmp/hm-build-target/release/examples")
CHUNK = 32

def list_hash(ctx, tag, ident, lst):
    pre = b"terp-hs-note-list-v2" + ctx + bytes([tag]) + ident + len(lst).to_bytes(2, "little")
    for i, (c, kc) in enumerate(lst):
        pre += bytes([c]) + i.to_bytes(2, "little") + bytes.fromhex(kc)
    return hashlib.sha256(pre).digest()

def gas_used(res):
    return int(res.get("gas_used", 0) or 0)

def main():
    addrs = dict(l.strip().split("=", 1) for l in open(os.path.join(HOME, "addresses.env")) if "=" in l)
    OWNER = addrs["OWNER_ADDR"]
    R["chain"] = {"chain_id": CHAIN_ID, "rpc": RPC, "rest": REST, "lane": LANE, "start_height": height()}
    R["addresses"] = addrs
    lane_dir = open(os.path.join(HOME, "lane.data")).read().strip()
    custody_pub = open(os.path.join(lane_dir, "custody.pub")).read().strip()

    # 1. in-memory identities
    evm_priv = secrets.randbelow(N - 1) + 1
    pt = pmul(evm_priv)
    evm_addr = kec(pt[0].to_bytes(32, "big") + pt[1].to_bytes(32, "big"))[12:]
    sol_sk = Ed25519PrivateKey.generate()
    sol_pk = sol_sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    ids = {"evm": (0x01, evm_addr, evm_addr.hex()), "sol": (0x02, sol_pk, "sol:" + sol_pk.hex())}
    R["identities"] = {"evm_address": "0x" + evm_addr.hex(), "sol_pubkey_b58": b58(sol_pk),
                       "note": "private keys and note secrets held in memory only"}

    # 2. denomination eligibility: EVM 200008 TERP -> 43 notes (2 chunks), Sol 3.5 TERP -> 2 notes + 0.5 unallocated
    comm = os.path.join(RUN, "communities", "mvk"); os.makedirs(comm, exist_ok=True)
    open(os.path.join(comm, "mvk.csv"), "w").write("addr,amount\n0x%s,200008\n%s,3.5\n" % (evm_addr.hex(), b58(sol_pk)))
    elig = os.path.join(RUN, "eligibility")
    subprocess.run([ELIG_BIN, os.path.join(RUN, "communities"), elig, "--decimals", "mvk=6", "--denominations"],
                   check=True, capture_output=True)
    ntree = json.load(open(os.path.join(elig, "notes", "tree.json")))
    nsum = json.load(open(os.path.join(elig, "notes", "summary.json")))
    paths = [json.loads(l) for l in open(os.path.join(elig, "notes", "note_paths.jsonl"))]
    notes = {k: sorted([p for p in paths if p["identity"] == ids[k][2]], key=lambda p: p["note_index"]) for k in ids}
    counts = {k: len(v) for k, v in notes.items()}
    R["eligibility"] = {"note_root_hex": ntree["root_hex"], "depth": ntree["depth"], "notes": ntree["notes"], "per_identity": counts,
                        "summary": nsum}
    step("note_eligibility_tree", ntree["depth"] == 17 and counts == {"evm": 43, "sol": 2},
         "root %s notes %s per identity %s" % (ntree["root_hex"], ntree["notes"], counts))

    # 3. store + instantiate
    res = rec("store", tx("owner", 250000000, "wasm", "store", WASM))
    code_id = attr(res, "code_id"); R["code_id"] = code_id
    R["wasm_sha256"] = hashlib.sha256(open(WASM, "rb").read()).hexdigest()
    step("store", res.get("code") == 0 and code_id, "code_id %s wasm %s" % (code_id, R["wasm_sha256"]))
    wavs = json.loads(subprocess.run([WAVS_BIN], capture_output=True, text=True, check=True).stdout)
    init = {"genesis_root": b64(bytes(32)), "genesis_label": "headstash-notes-e2e",
            "token_strategy": {"existing_fungible": {"proof": h2b64(DENOM), "raw": "uterp"}}, "wavs": wavs}
    res = rec("instantiate", tx("owner", 80000000, "wasm", "instantiate", code_id, json.dumps(init),
                                "--label", "headstash-notes-e2e", "--admin", OWNER, amount="5030000000uterp"))  # funds the 5000 TERP note payout
    contract = attr(res, "_contract_address"); R["contract"] = contract
    step("instantiate", res.get("code") == 0 and contract, contract or res.get("raw_log", ""))

    ctx = pallas_rand(); R["ctx_hex"] = ctx.hex()
    cut = height() + CUT_AHEAD; R["cutoff_height"] = cut
    classes = {}
    for k in ids:
        for p in notes[k]: classes[p["class"]] = classes.get(p["class"], 0) + 1
    amounts = [{"amount": DENOMS[c], "cap": n} for c, n in sorted(classes.items())]
    def campaign(alloc, vk_id=3, vk_hash=NOTE_VK_HASH):
        return {"set_epoch_campaign": {"campaign": {
            "ctx": b64(ctx), "backend": "halo2", "cutoff_height": cut, "bech32_hrp": "terp", "denom": h2b64(DENOM),
            "amounts": amounts, "reserved": [], "trusted": [h2b64(custody_pub)], "trusted_verify_threshold": 1,
            "trusted_sign_threshold": 1, "max_rows": 8, "allocation_root": b64(alloc), "root_id": 0, "vk_id": vk_id,
            "vk_hash": h2b64(vk_hash), "registration": {"eligibility_root": h2b64(ntree["root_hex"]), "eligibility_depth": 17,
                             "min_anonymity_set": 2, "anchor_start_height": 1, "anchor_interval": 1}}}}

    # NEG: vk_id 2 campaign rejected for a new campaign
    res = tx("owner", 5000000, "wasm", "execute", contract, json.dumps(campaign(bytes(32), 2, VK_HASH)))
    R["negatives"]["vk2_campaign"] = "code %s %s" % (res.get("code"), res.get("raw_log", "")[:240])
    step("neg_vk2_campaign_rejected", res.get("code") != 0 and "retired" in res.get("raw_log", ""), R["negatives"]["vk2_campaign"])
    res = rec("set_campaign", tx("owner", 5000000, "wasm", "execute", contract, json.dumps(campaign(bytes(32)))))
    oc = smart(contract, {"epoch_campaign": {}})
    step("set_campaign_vk3", res.get("code") == 0 and oc.get("vk_id") == 3, "vk_id %s amounts %s" % (oc.get("vk_id"), amounts))

    # 4. note secrets + key commitments
    sec = {k: [pallas_rand() for _ in notes[k]] for k in ids}
    kcs = {k: [subprocess.run([EX + "/membership_key_com"], input=s.hex(), capture_output=True, text=True, check=True).stdout.strip()
               for s in sec[k]] for k in ids}
    lists = {k: [(p["class"], kcs[k][i]) for i, p in enumerate(notes[k])] for k in ids}
    def wire(lst): return [{"class": c, "key_com": h2b64(kc)} for c, kc in lst]
    def signer(k): return {"evm": {"address": b64(evm_addr)}} if k == "evm" else {"sol": {"pubkey": b64(sol_pk)}}
    def sign(k, lst):
        lh = list_hash(ctx, ids[k][0], ids[k][1], lst).hex()
        if k == "evm":
            return evm_sign(evm_priv, eip191("headstash-register-notes-v2:%s:%s:%d" % (ctx.hex(), lh, len(lst))))
        return sol_sk.sign(("headstash-register-notes-sol-v2:%s:%s:%d" % (ctx.hex(), lh, len(lst))).encode())
    def chunk_msg(k, lst, start, sig, rows):
        return {"register_notes": {"signer": signer(k), "signature": b64(sig) if sig else None, "list": wire(lst), "start": start,
                "eligibility": [{"index": r["leaf_index"], "path": [h2b64(x) for x in r["path"]]} for r in rows]}}

    # NEG: duplicate key_com inside one signed list (Sol, before its real registration)
    dl = [(lists["sol"][0][0], kcs["sol"][0]), (lists["sol"][1][0], kcs["sol"][0])]
    res = tx("stranger", 20000000, "wasm", "execute", contract, json.dumps(chunk_msg("sol", dl, 0, sign("sol", dl), notes["sol"])))
    R["negatives"]["duplicate_key_com"] = "code %s %s" % (res.get("code"), res.get("raw_log", "")[:240])
    step("neg_duplicate_key_com_in_list", res.get("code") != 0 and "duplicate key commitment" in res.get("raw_log", ""),
         R["negatives"]["duplicate_key_com"])

    # 5. EVM: multi-chunk RegisterNotes (32 + 11), relayed by a stranger
    sig = sign("evm", lists["evm"])
    gas = []
    res = rec("register_notes_evm_0", tx("stranger", 60000000, "wasm", "execute", contract,
              json.dumps(chunk_msg("evm", lists["evm"], 0, sig, notes["evm"][:CHUNK]))))
    gas.append((CHUNK, gas_used(res)))
    step("register_notes_evm_chunk0", res.get("code") == 0, "32 notes gas %d (%d/note) %s" % (gas_used(res), gas_used(res) // CHUNK, res.get("raw_log", "")[:160]))
    # NEG: chunk 2 with a list that differs from the signed one
    bad = list(lists["evm"]); bad[40] = (bad[40][0], subprocess.run([EX + "/membership_key_com"], input=pallas_rand().hex(), capture_output=True, text=True, check=True).stdout.strip())
    res = tx("stranger", 60000000, "wasm", "execute", contract, json.dumps(chunk_msg("evm", bad, CHUNK, None, notes["evm"][CHUNK:])))
    R["negatives"]["mismatched_list_hash"] = "code %s %s" % (res.get("code"), res.get("raw_log", "")[:240])
    step("neg_mismatched_list_hash", res.get("code") != 0 and "list hash" in res.get("raw_log", ""), R["negatives"]["mismatched_list_hash"])
    rest = len(lists["evm"]) - CHUNK
    res = rec("register_notes_evm_1", tx("stranger", 60000000, "wasm", "execute", contract,
              json.dumps(chunk_msg("evm", lists["evm"], CHUNK, None, notes["evm"][CHUNK:]))))
    gas.append((rest, gas_used(res)))
    step("register_notes_evm_chunk1", res.get("code") == 0, "%d notes gas %d (%d/note)" % (rest, gas_used(res), gas_used(res) // rest))
    nl = smart(contract, {"note_list": {"identity": ids["evm"][2]}})
    step("evm_note_list_complete", nl and nl.get("n") == 43 and nl.get("next") == 43, json.dumps(nl))

    # NEG: duplicate key_com across identities (Sol list reuses an EVM key_com)
    xl = [(lists["sol"][0][0], kcs["evm"][0]), lists["sol"][1]]
    res = tx("stranger", 20000000, "wasm", "execute", contract, json.dumps(chunk_msg("sol", xl, 0, sign("sol", xl), notes["sol"])))
    R["negatives"]["duplicate_key_com_cross_identity"] = "code %s %s" % (res.get("code"), res.get("raw_log", "")[:240])
    step("neg_duplicate_key_com_cross_identity", res.get("code") != 0 and "key commitment" in res.get("raw_log", ""),
         R["negatives"]["duplicate_key_com_cross_identity"])
    res = rec("register_notes_sol", tx("stranger", 20000000, "wasm", "execute", contract,
              json.dumps(chunk_msg("sol", lists["sol"], 0, sign("sol", lists["sol"]), notes["sol"]))))
    gas.append((2, gas_used(res)))
    step("register_notes_sol", res.get("code") == 0, "2 notes gas %d %s" % (gas_used(res), res.get("raw_log", "")[:160]))
    R["gas_per_note"] = [{"notes": n, "gas_used": g, "per_note": g // n} for n, g in gas]
    # marginal: (chunk0 - chunk1) / (32 - 11)
    R["gas_marginal_per_note"] = (gas[0][1] - gas[1][1]) // (gas[0][0] - gas[1][0])
    log("gas", R["gas_per_note"], "marginal", R["gas_marginal_per_note"]); save()

    # 6. anchor
    rt = smart(contract, {"registration_tree": {}})
    while height() < int(rt.get("next_anchor_height", 0)): time.sleep(1)
    res = rec("accept_anchor", tx("stranger", 5000000, "wasm", "execute", contract, json.dumps({"accept_anchor": {}})))
    rt = smart(contract, {"registration_tree": {}})
    anchor = base64.b64decode(rt["anchors"][-1]["root"]); R["anchor_hex"] = anchor.hex()
    step("accept_anchor", res.get("code") == 0 and rt["anchors"][-1]["leaf_count"] == 45, "anchor %s leaves %s" % (anchor.hex(), rt["anchors"][-1]["leaf_count"]))
    res = rec("set_campaign_anchor", tx("owner", 5000000, "wasm", "execute", contract, json.dumps(campaign(anchor))))
    step("campaign_alloc_anchor", res.get("code") == 0, res.get("raw_log", "")[:200])
    res = rec("escrow_register", tx("owner", 3000000, "hashmerchant", "register-contract", contract, "headstash-claim", "bank", "1000000uterp"))
    step("escrow_register_contract", res.get("code") == 0, res.get("raw_log", "")[:200])

    # 7. one fresh temporary wallet per claimed note, fee-granted by the owner
    picks = [("evm", 0), ("evm", 41), ("evm", 42), ("sol", 0), ("sol", 1)]
    wallets = []
    for j, _ in enumerate(picks):
        name = "tmpclaim%d" % j
        terpd("keys", "add", name, "--keyring-backend", "test", "--home", HOME, "--output", "json")  # mnemonic discarded
        wallets.append(terpd("keys", "show", name, "-a", "--keyring-backend", "test", "--home", HOME).strip())
    step("fresh_claim_wallets", len(set(wallets)) == len(wallets) and all(bal(w) == 0 for w in wallets), " ".join(wallets))
    R["claim_wallets"] = wallets

    def witness(k, i, dest, amount=None):
        pth = smart(contract, {"registration_path": {"key_com": h2b64(kcs[k][i]), "anchor": b64(anchor)}})
        return json.dumps({"secret_hex": sec[k][i].hex(), "ctx_hex": ctx.hex(), "root_hex": anchor.hex(), "destination": dest,
                           "amount": str(amount if amount is not None else pth["amount"]), "index": pth["index"], "siblings_hex": pth["siblings"]}), pth
    def prove(wit):
        out = subprocess.run([NOTE_EX + "/prove_note_witness", "/dev/stdin"], input=wit, capture_output=True, text=True)
        return (json.loads(out.stdout) if out.returncode == 0 else None), out.stderr.strip()[-200:]
    proofs = []
    for j, (k, i) in enumerate(picks):
        wit, pth = witness(k, i, wallets[j])
        t0 = time.time(); pr, err = prove(wit)
        ok = pr is not None and pr["vk_id"] == 3 and len(pth["siblings"]) == 17 and int(pth["amount"]) == DENOMS[lists[k][i][0]]
        step("prove_note_%s_%d" % (k, i), ok, "amount %s k %s bytes %s %.1fs" % (pth.get("amount"), pr and pr["k"], pr and pr["proof_bytes"], time.time() - t0) if pr else err)
        if not pr: return
        proofs.append(pr)
    R["proofs"] = [{"body": p["body"], "proof_bytes": p["proof_bytes"], "digest": hashlib.sha256(bytes.fromhex(p["proof_hex"])).hexdigest()} for p in proofs]

    # 8. lane campaign + claims
    subprocess.run([os.path.join(HERE, "bootstrap-chain.sh"), "lane"], check=True)
    body = json.dumps({"backend": "halo2", "ctx_hex": ctx.hex(), "allocation_root_hex": anchor.hex(), "bech32_hrp": "terp",
            "denom_hex": DENOM, "proof_len": proofs[0]["proof_bytes"], "cutoff_height": cut, "amounts": amounts,
            "trusted_hex": [custody_pub], "trusted_verify_threshold": 1, "trusted_sign_threshold": 1, "max_rows": 8,
            "reserved_hex": [], "vk_id": 3, "vk_hash_hex": NOTE_VK_HASH, "registration_anchor_hex": anchor.hex()}).encode()
    r = opsign_post("/ve/headstash/campaign", body)
    step("lane_campaign_vk3", r.status_code == 200, "%d %s" % (r.status_code, r.text[:200]))
    def claim(proof_hex, message):
        return requests.post(LANE + "/ve/headstash/claims", json={"claims": [{"proof_hex": proof_hex, "message": message,
                             "denom_hex": DENOM, "vk_id": 3}]}, timeout=600)
    # NEG: wrong-class amount (a) proven body with its amount swapped for another class, (b) a non-denomination witness
    parts = proofs[0]["body"].split(":"); parts[5] = str(DENOMS[4])
    r = claim(proofs[0]["proof_hex"], ":".join(parts))
    wa = "%d %s" % (r.status_code, r.text[:200])
    wit, _ = witness("evm", 0, wallets[0], amount=3_000_000)
    pr_bad, err_bad = prove(wit)
    R["negatives"]["wrong_class_amount"] = {"lane_swapped_class": wa, "prover_non_denomination": err_bad or "proved"}
    step("neg_wrong_class_amount", (r.status_code >= 400 or "error" in r.text or "rejected" in r.text) and pr_bad is None,
         json.dumps(R["negatives"]["wrong_class_amount"])[:300])
    # NEG: same-set amount swap: a proven 1 TERP note body re-claimed as 2 TERP (both in the campaign set)
    one = [pr for pr in proofs if pr["body"].split(":")[5] == str(1_000_000)]
    if one and 2_000_000 in [int(a["amount"]) for a in amounts]:
        parts = one[0]["body"].split(":"); parts[5] = str(2_000_000)
        r = claim(one[0]["proof_hex"], ":".join(parts))
        R["negatives"]["same_set_amount_swap"] = "%d %s" % (r.status_code, r.text[:200])
        step("neg_same_set_amount_swap", r.status_code >= 400 or "error" in r.text or "rejected" in r.text,
             R["negatives"]["same_set_amount_swap"])
    else:
        step("neg_same_set_amount_swap", False, "no 1 TERP proof or 2 TERP not in campaign amounts")
    before = {w: bal(w) for w in wallets}
    for j, pr in enumerate(proofs):
        r = claim(pr["proof_hex"], pr["body"])
        R.setdefault("lane_claims", []).append("%d %s" % (r.status_code, r.text[:300]))
        step("lane_claim_%d" % j, r.status_code == 200 and "error" not in r.text, R["lane_claims"][-1][:200])
    # NEG: replayed nullifier (fresh proof of an already claimed note to a different wallet)
    wit, _ = witness("evm", 0, addrs["STRANGER_ADDR"])
    pr2, _ = prove(wit)
    r = claim(pr2["proof_hex"], pr2["body"]) if pr2 else None
    R["negatives"]["replayed_nullifier"] = "%d %s" % (r.status_code, r.text[:200]) if r is not None else "prove failed"
    step("neg_replayed_nullifier", r is not None and "nullifier" in r.text and "already" in r.text, R["negatives"]["replayed_nullifier"])
    save()

    # 9. stage (pays 0) -> seal -> epoch payout
    rows = []
    for pr in proofs:
        p = pr["body"].split(":")  # headstash-note-claim-v2:ctx:root:nullifier:dest:amount
        rows.append({"nullifier": h2b64(p[3]), "denom": h2b64(DENOM), "destination": p[4], "amount": int(p[5]), "anchor": b64(anchor)})
    digests = [b64(bytes.fromhex(x["digest"])) for x in R["proofs"]]
    stage = json.dumps({"stage_epoch_batch": {"digests": digests, "rows": rows}})
    res = rec("stage", tx("owner", 8000000, "wasm", "execute", contract, stage))
    step("stage_pays_nothing", res.get("code") == 0 and all(bal(w) == before[w] for w in wallets),
         "code %s minted %s %s" % (res.get("code"), attr(res, "minted"), res.get("raw_log", "")[:200]))
    while height() < cut + 1: time.sleep(1)
    r = opsign_post("/ve/headstash/seal", json.dumps({"height": cut}).encode())
    R["seal"] = {"status": r.status_code, "body": r.text[:400], "chain_height_at_seal": height()}
    step("seal", r.status_code == 200, R["seal"]["body"][:200])
    want = {wallets[j]: DENOMS[lists[k][i][0]] for j, (k, i) in enumerate(picks)}
    end = time.time() + 120; er = {}
    while time.time() < end:
        try: er = smart(contract, {"epoch_root": {"algo": "headstash-accepted"}})
        except Exception: er = {}
        if er.get("root") and all(bal(w) > before[w] for w in wallets): break
        time.sleep(2)
    after = {w: bal(w) for w in wallets}
    R["payout"] = {w: {"want": want[w], "got": after[w] - before[w]} for w in wallets}; R["epoch_root"] = er
    paid_h = None
    for hgt in range(R["tx"]["stage"]["height"], height() + 1):
        br = requests.get(RPC + "/block_results?height=%d" % hgt, timeout=10).json()["result"]
        for e in (br.get("finalize_block_events") or []):
            kv = {a["key"]: a["value"] for a in e.get("attributes", [])}
            if kv.get("_contract_address") == contract and kv.get("minted") not in (None, "0"):
                paid_h = hgt; R["payout_event"] = {"height": hgt, "type": e["type"], "attrs": kv}
        if paid_h: break
    step("epoch_payout_exact_per_note", paid_h is not None and all(after[w] - before[w] == want[w] for w in wallets),
         "payout height %s %s" % (paid_h, json.dumps(R["payout"])))
    res = tx("owner", 8000000, "wasm", "execute", contract, stage)
    R["negatives"]["double_payout_restage"] = "code %s minted %s %s" % (res.get("code"), attr(res, "minted"), res.get("raw_log", "")[:200])
    step("neg_double_payout_restage", (res.get("code") != 0 or attr(res, "minted") in ("0", None)) and all(bal(w) == after[w] for w in wallets),
         R["negatives"]["double_payout_restage"])

    # 10. fee grants: each temporary wallet moves its note with fees paid by the owner
    fg = []
    for j, w in enumerate(wallets):
        g = tx("owner", 300000, "feegrant", "grant", OWNER, w, "--spend-limit", "200000uterp")
        a = ["tx", "bank", "send", w, w, "1uterp", "--home", HOME, "--keyring-backend", "test", "--chain-id", CHAIN_ID, "--node", NODE,
             "--gas", "200000", "--gas-prices", "0.025uterp", "--fee-granter", OWNER, "-b", "sync", "-y", "-o", "json"]
        p = subprocess.run([TERPD, *a], capture_output=True, text=True)
        try:
            s = json.loads(p.stdout); s = wait_tx(s["txhash"]) if s.get("code", 0) == 0 else s
        except Exception:
            s = {"code": -1, "raw_log": (p.stdout + p.stderr)[-200:]}
        fg.append({"wallet": w, "grant_code": g.get("code"), "send_code": s.get("code"), "balance": bal(w)})
    R["fee_grants"] = fg
    step("fee_granted_claim_wallets", all(x["grant_code"] == 0 and x["send_code"] == 0 and x["balance"] == after[x["wallet"]] for x in fg),
         json.dumps(fg)[:300])
    save()

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
