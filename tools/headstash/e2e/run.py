#!/usr/bin/env python3
"""Headstash membership-vk local e2e harness (loopback only).

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
ELIG_BIN = os.environ.get("ELIG_BIN", "/tmp/headstash-membership-vk-target/release/headstash-eligibility")
EX = os.environ.get("ZK_EX", "/tmp/zk-standalone/target/release/examples")
WAVS_BIN = os.environ.get("WAVS_BIN", "/tmp/cwhs-native-target/release/examples/print_wavs")
WASM = os.environ.get("WASM", "/tmp/cw_headstash_opt.wasm")
OPSIGN = os.path.join(HERE, "..", "..", "hash-market", "scripts", "headstash-operator-sign.py")
VK_HASH = "94d4326108e28451829b68d64df58cf975fafdb874a5a554f3cbe8931eefed11"
DENOM = "1d56a5879c78333d517942a36e7ea0932e7ffa1a1027e439279e8517580ea706"
CUT = int(os.environ.get("CUT", "100"))
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
    pub = subprocess.run(["python3", OPSIGN, "--key", key, "--pubkey-only"], capture_output=True, text=True, check=True).stdout.strip()
    sig = subprocess.run(["python3", OPSIGN, "--key", key, "--method", "POST", "--path", path, "--body", f], capture_output=True, text=True, check=True).stdout.strip()
    return requests.post(LANE + path, data=body_bytes, timeout=600, headers={
        "content-type": "application/json", "x-hash-market-pubkey": pub, "x-hash-market-signature": sig})

def main():
    addrs = dict(l.strip().split("=", 1) for l in open(os.path.join(HOME, "addresses.env")) if "=" in l)
    OWNER, R1, R2 = addrs["OWNER_ADDR"], addrs["RECIPIENT1_ADDR"], addrs["RECIPIENT2_ADDR"]
    R["chain"] = {"chain_id": CHAIN_ID, "rpc": RPC, "rest": REST, "lane": LANE, "relay": RELAY_BIND, "start_height": height()}
    R["addresses"] = addrs
    lane_dir = open(os.path.join(HOME, "lane.data")).read().strip()
    custody_pub = open(os.path.join(lane_dir, "custody.pub")).read().strip()

    # 1. in-memory identities
    evm_priv = secrets.randbelow(N - 1) + 1
    pt = pmul(evm_priv)
    evm_addr = kec(pt[0].to_bytes(32, "big") + pt[1].to_bytes(32, "big"))[12:]
    sol_sk = Ed25519PrivateKey.generate()
    sol_pk = sol_sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    sec = {"evm": pallas_rand(), "sol": pallas_rand()}
    kc = {}
    for k, s in sec.items():
        kc[k] = subprocess.run([EX + "/membership_key_com"], input=s.hex(), capture_output=True, text=True, check=True).stdout.strip()
    R["identities"] = {"evm_address": "0x" + evm_addr.hex(), "sol_pubkey_b58": b58(sol_pk), "key_com": kc,
                       "note": "private keys and membership secrets held in memory only"}
    log("identities", R["identities"])

    # 2. eligibility (4bc3c3b builder: exact decimals + floor budget)
    comm = os.path.join(RUN, "communities", "mvk"); os.makedirs(comm, exist_ok=True)
    open(os.path.join(comm, "mvk.csv"), "w").write("addr,amount\n0x%s,1.5\n%s,2.5\n" % (evm_addr.hex(), b58(sol_pk)))
    elig = os.path.join(RUN, "eligibility")
    subprocess.run([ELIG_BIN, os.path.join(RUN, "communities"), elig, "--decimals", "mvk=6", "--budget", "mvk=4000000"], check=True)
    tree = json.load(open(os.path.join(elig, "tree.json")))
    summ = json.load(open(os.path.join(elig, "summary.json")))
    amt = {}
    for e in tree["entries"]:
        if "address_hex" in e: amt["evm"] = int(e["amount"])
        else: amt["sol"] = int(e["amount"])
    R["eligibility"] = {"root_hex": tree["root_hex"], "depth": tree["depth"], "amounts_uterp": amt,
                        "dust_uterp": summ.get("total_dust_uterp")}
    step("eligibility_exact_decimals", amt == {"evm": 1500000, "sol": 2500000}, json.dumps(R["eligibility"]))

    # 3. store + instantiate
    res = rec("store", tx("owner", 250000000, "wasm", "store", WASM))
    code_id = attr(res, "code_id"); R["code_id"] = code_id
    step("store", res.get("code") == 0 and code_id, "code_id %s" % code_id)
    wavs = json.loads(subprocess.run([WAVS_BIN], capture_output=True, text=True, check=True).stdout)
    init = {"genesis_root": b64(bytes(32)), "genesis_label": "headstash-mvk-e2e",
            "token_strategy": {"existing_fungible": {"proof": h2b64(DENOM), "raw": "uterp"}}, "wavs": wavs}
    res = rec("instantiate", tx("owner", 80000000, "wasm", "instantiate", code_id, json.dumps(init),
                                "--label", "headstash-mvk-e2e", "--admin", OWNER, amount="5000000uterp"))
    contract = attr(res, "_contract_address"); R["contract"] = contract
    step("instantiate", res.get("code") == 0 and contract, contract or res.get("raw_log", ""))

    ctx = pallas_rand(); R["ctx_hex"] = ctx.hex()
    def campaign(alloc):
        return {"set_epoch_campaign": {"campaign": {
            "ctx": b64(ctx), "backend": "halo2", "cutoff_height": CUT, "bech32_hrp": "terp", "denom": h2b64(DENOM),
            "amounts": [{"amount": 1500000, "cap": 1}, {"amount": 2500000, "cap": 1}], "reserved": [],
            "trusted": [h2b64(custody_pub)], "trusted_verify_threshold": 1, "trusted_sign_threshold": 1,
            "max_rows": 4, "allocation_root": b64(alloc), "root_id": 0, "vk_id": 2, "vk_hash": h2b64(VK_HASH),
            "registration": {"eligibility_root": h2b64(tree["root_hex"]), "eligibility_depth": tree["depth"],
                             "min_anonymity_set": 2, "anchor_start_height": 1, "anchor_interval": 1}}}}
    res = rec("set_campaign", tx("owner", 5000000, "wasm", "execute", contract, json.dumps(campaign(bytes(32)))))
    oc = smart(contract, {"epoch_campaign": {}})
    step("set_campaign_vk2", res.get("code") == 0 and oc.get("vk_id") == 2 and oc.get("vk_hash") == h2b64(VK_HASH),
         "vk_id %s vk_hash %s" % (oc.get("vk_id"), VK_HASH))

    # 4. relay (batches EVM + Sol in one tx)
    env = dict(os.environ, HEADSTASH_RELAY_BIND=RELAY_BIND, HEADSTASH_ELIGIBILITY=os.path.join(elig, "tree.json"),
               HEADSTASH_RELAY_HOME=HOME, HEADSTASH_RELAY_FROM="relayer", HEADSTASH_KEYRING_BACKEND="test",
               HEADSTASH_REST=REST, HEADSTASH_CONTRACT=contract, HEADSTASH_CHAIN_ID=CHAIN_ID, TERPD=TERPD,
               HEADSTASH_NODE=NODE, HEADSTASH_FLUSH_SECS="6", HEADSTASH_MAX_BATCH="25")
    relay_log = open(os.path.join(RUN, "relay.log"), "w")
    relay = subprocess.Popen([RELAY_BIN], env=env, stdout=relay_log, stderr=subprocess.STDOUT)
    open(os.path.join(HOME, "relay.pid"), "w").write(str(relay.pid))
    try:
        for _ in range(60):
            if "relay_up" in open(os.path.join(RUN, "relay.log")).read(): break
            if relay.poll() is not None: break
            time.sleep(1)
        rl = open(os.path.join(RUN, "relay.log")).read()
        step("relay_up", "relay_up" in rl, rl.strip().splitlines()[-1] if rl.strip() else "")
        if "relay_up" not in rl: return
        RELAY = "http://" + RELAY_BIND
        def evm_req(keycom, amount, priv=evm_priv):
            msg = "headstash-register-v1:%s:%s:%d" % (ctx.hex(), keycom, amount)
            return {"ctx_hex": ctx.hex(), "key_com_hex": keycom, "amount": amount, "kind": "evm",
                    "signature_hex": evm_sign(priv, eip191(msg)).hex()}
        def sol_req(keycom, amount):
            msg = "headstash-register-sol-v1:%s:%s:%d" % (ctx.hex(), keycom, amount)
            return {"ctx_hex": ctx.hex(), "key_com_hex": keycom, "amount": amount, "kind": "sol",
                    "signature_hex": sol_sk.sign(msg.encode()).hex(), "pubkey_hex": sol_pk.hex()}
        ids = {}
        for k, body in (("evm", evm_req(kc["evm"], amt["evm"])), ("sol", sol_req(kc["sol"], amt["sol"]))):
            r = requests.post(RELAY + "/register", json=body, timeout=30)
            ids[k] = r.json().get("id"); log("register", k, r.status_code, r.text[:200])
        st = {}
        end = time.time() + 120
        while time.time() < end:
            st = {k: requests.get(RELAY + "/register/status/%s" % i, timeout=10).json() for k, i in ids.items()}
            if all(v.get("status") in ("registered", "failed") for v in st.values()): break
            time.sleep(2)
        log("relay status", st)
        hashes = {v.get("txhash") for v in st.values()}
        ok = all(v.get("status") == "registered" for v in st.values()) and len(hashes) == 1
        th = st["evm"].get("txhash")
        R["tx"]["batched_registration"] = {"txhash": th, "height": int(st["evm"].get("height", 0) or 0), "code": 0}
        if th:
            t = wait_tx(th); msgs = t.get("tx", {}).get("body", {}).get("messages", [])
            kinds = [list(json.loads(json.dumps(m.get("msg"))).keys())[0] if isinstance(m.get("msg"), dict) else "?" for m in msgs]
            R["tx"]["batched_registration"]["messages"] = kinds
            ok = ok and "register_headstash" in kinds and "register_headstash_sol" in kinds
        step("batched_registration_evm_sol_one_tx", ok, json.dumps(R["tx"]["batched_registration"]))
        save()

        # 5. registration negatives
        r = requests.post(RELAY + "/register", json=evm_req(kc["evm"], amt["evm"]), timeout=30)
        dup = r.text
        if r.status_code == 200 and "id" in r.json():
            for _ in range(60):
                s = requests.get(RELAY + "/register/status/%s" % r.json()["id"], timeout=10).json()
                if s.get("status") in ("registered", "failed"): break
                time.sleep(2)
            dup = json.dumps(s)
        R["negatives"]["duplicate_registration_relay"] = dup
        # relay is idempotent: a resubmission maps to the existing request, no second tx
        step("neg_duplicate_registration_relay", (ids["evm"] in dup and th in dup) or "failed" in dup or r.status_code >= 400,
             "same id, same txhash (no new tx): " + dup[:200])
        other = secrets.randbelow(N - 1) + 1
        bad = evm_req(kc["evm"], amt["evm"], priv=other)
        r = requests.post(RELAY + "/register", json=bad, timeout=30)
        R["negatives"]["wrong_signature_relay"] = "%d %s" % (r.status_code, r.text[:200])
        step("neg_wrong_signature_relay", r.status_code >= 400, R["negatives"]["wrong_signature_relay"])
        # direct on-chain duplicate (bypasses relay pre-checks)
        line = [json.loads(l) for l in open(os.path.join(elig, "paths.jsonl"))]
        ev = [l for l in line if l["identity"] == evm_addr.hex()][0]
        msg = "headstash-register-v1:%s:%s:%d" % (ctx.hex(), kc["evm"], amt["evm"])
        direct = {"register_headstash": {"key_com": h2b64(kc["evm"]), "amount": amt["evm"],
                  "signature": b64(evm_sign(evm_priv, eip191(msg))), "eligibility_index": ev["index"],
                  "eligibility_path": [h2b64(x) for x in ev["path"]]}}
        res = tx("stranger", 5000000, "wasm", "execute", contract, json.dumps(direct))
        R["negatives"]["duplicate_registration_chain"] = "code %s %s" % (res.get("code"), res.get("raw_log", "")[:200])
        step("neg_duplicate_registration_chain", res.get("code") != 0, R["negatives"]["duplicate_registration_chain"])
    finally:
        relay.terminate(); relay.wait(10)

    # 6. anchor
    rt = smart(contract, {"registration_tree": {}})
    while height() < int(rt.get("next_anchor_height", 0)): time.sleep(1)
    res = rec("accept_anchor", tx("stranger", 5000000, "wasm", "execute", contract, json.dumps({"accept_anchor": {}})))
    rt = smart(contract, {"registration_tree": {}})
    anchor = base64.b64decode(rt["anchors"][-1]["root"]); R["anchor_hex"] = anchor.hex()
    step("accept_anchor", res.get("code") == 0 and rt["anchors"][-1]["leaf_count"] == 2, "anchor %s count %s" % (anchor.hex(), rt["count"]))
    res = rec("set_campaign_anchor", tx("owner", 5000000, "wasm", "execute", contract, json.dumps(campaign(anchor))))
    step("campaign_alloc_anchor", res.get("code") == 0, res.get("raw_log", "")[:200])
    res = rec("escrow_register", tx("owner", 3000000, "hashmerchant", "register-contract", contract, "headstash-claim", "bank", "1000000uterp"))
    step("escrow_register_contract", res.get("code") == 0, res.get("raw_log", "")[:200])

    # 7. proofs (browser prover path prove_witness_json, K=11)
    dest = {"evm": R1, "sol": R2}
    proofs = {}
    for k in ("evm", "sol"):
        pth = smart(contract, {"registration_path": {"key_com": h2b64(kc[k]), "anchor": b64(anchor)}})
        wit = json.dumps({"secret_hex": sec[k].hex(), "ctx_hex": ctx.hex(), "root_hex": anchor.hex(), "destination": dest[k],
                          "amount": pth["amount"], "index": pth["index"], "siblings_hex": pth["siblings"]})
        t0 = time.time()
        out = subprocess.run([EX + "/prove_membership_witness", "/dev/stdin"], input=wit, capture_output=True, text=True)
        if out.returncode != 0: step("prove_" + k, False, out.stderr[-300:]); return
        pr = json.loads(out.stdout); proofs[k] = pr
        step("prove_" + k, pr["k"] == 11 and pr["vk_id"] == 2, "k %s vk_id %s bytes %s %.1fs dest %s" % (pr["k"], pr["vk_id"], pr["proof_bytes"], time.time() - t0, dest[k]))
    R["proofs"] = {k: {"k": v["k"], "vk_id": v["vk_id"], "proof_bytes": v["proof_bytes"], "body": v["body"],
                       "digest": hashlib.sha256(bytes.fromhex(v["proof_hex"])).hexdigest()} for k, v in proofs.items()}

    # 8. lane campaign + claims
    subprocess.run([os.path.join(HERE, "bootstrap-chain.sh"), "lane"], check=True)
    body = json.dumps({"backend": "halo2", "ctx_hex": ctx.hex(), "allocation_root_hex": anchor.hex(), "bech32_hrp": "terp",
            "denom_hex": DENOM, "proof_len": proofs["evm"]["proof_bytes"], "cutoff_height": CUT,
            "amounts": [{"amount": 1500000, "cap": 1}, {"amount": 2500000, "cap": 1}], "trusted_hex": [custody_pub],
            "trusted_verify_threshold": 1, "trusted_sign_threshold": 1, "max_rows": 4, "reserved_hex": [], "vk_id": 2,
            "vk_hash_hex": VK_HASH, "registration_anchor_hex": anchor.hex()}).encode()
    r = opsign_post("/ve/headstash/campaign", body)
    step("lane_campaign", r.status_code == 200, "%d %s" % (r.status_code, r.text[:200]))
    before = {"recipient1": bal(R1), "recipient2": bal(R2)}
    R["balances_before"] = before; log("before", before)
    for k in ("evm", "sol"):
        pr = proofs[k]
        r = requests.post(LANE + "/ve/headstash/claims", json={"claims": [{"proof_hex": pr["proof_hex"], "message": pr["body"],
                          "denom_hex": DENOM, "vk_id": 2}]}, timeout=600)
        R.setdefault("lane_claims", {})[k] = "%d %s" % (r.status_code, r.text[:300])
        step("lane_claim_" + k, r.status_code == 200 and "error" not in r.text, R["lane_claims"][k])
    r = requests.post(LANE + "/ve/headstash/claims", json={"claims": [{"proof_hex": proofs["evm"]["proof_hex"], "message": proofs["evm"]["body"], "denom_hex": DENOM, "vk_id": 2}]}, timeout=600)
    R["negatives"]["double_claim_digest"] = "%d %s" % (r.status_code, r.text[:200])
    step("neg_double_claim_same_proof", "already" in r.text, R["negatives"]["double_claim_digest"])
    # fresh proof, same secret -> same nullifier
    pth = smart(contract, {"registration_path": {"key_com": h2b64(kc["evm"]), "anchor": b64(anchor)}})
    wit = json.dumps({"secret_hex": sec["evm"].hex(), "ctx_hex": ctx.hex(), "root_hex": anchor.hex(), "destination": addrs["STRANGER_ADDR"],
                      "amount": pth["amount"], "index": pth["index"], "siblings_hex": pth["siblings"]})
    out = subprocess.run([EX + "/prove_membership_witness", "/dev/stdin"], input=wit, capture_output=True, text=True, check=True)
    pr2 = json.loads(out.stdout)
    r = requests.post(LANE + "/ve/headstash/claims", json={"claims": [{"proof_hex": pr2["proof_hex"], "message": pr2["body"], "denom_hex": DENOM, "vk_id": 2}]}, timeout=600)
    R["negatives"]["double_claim_nullifier"] = "%d %s" % (r.status_code, r.text[:200])
    step("neg_double_claim_new_proof_same_nullifier", "nullifier" in r.text and "already" in r.text, R["negatives"]["double_claim_nullifier"])
    save()

    # 9. stage (pays 0) -> seal -> sudo payout
    rows, digests = [], []
    for k in ("evm", "sol"):
        parts = proofs[k]["body"].split(":")  # headstash-claim-v1:ctx:root:nullifier:dest:amount
        rows.append({"nullifier": h2b64(parts[3]), "denom": h2b64(DENOM), "destination": parts[4], "amount": int(parts[5]), "anchor": b64(anchor)})
        digests.append(b64(bytes.fromhex(R["proofs"][k]["digest"])))
    stage = json.dumps({"stage_epoch_batch": {"digests": digests, "rows": rows}})
    res = rec("stage", tx("owner", 5000000, "wasm", "execute", contract, stage))
    step("stage_pays_nothing", res.get("code") == 0 and attr(res, "minted") in ("0", None) and bal(R1) == before["recipient1"],
         "code %s minted %s %s" % (res.get("code"), attr(res, "minted"), res.get("raw_log", "")[:200]))
    while height() < CUT + 1: time.sleep(1)
    r = opsign_post("/ve/headstash/seal", json.dumps({"height": CUT}).encode())
    R["seal"] = {"status": r.status_code, "body": r.text[:400], "chain_height_at_seal": height()}
    step("seal", r.status_code == 200, R["seal"]["body"][:200])
    paid_h = None; er = {}
    end = time.time() + 120
    while time.time() < end:
        try: er = smart(contract, {"epoch_root": {"algo": "headstash-accepted"}})
        except Exception: er = {}
        if er.get("root") and bal(R1) > before["recipient1"]: break
        time.sleep(2)
    after = {"recipient1": bal(R1), "recipient2": bal(R2)}
    R["balances_after"] = after; R["epoch_root"] = er
    # find the block whose finalize events carry the sudo payout
    for hgt in range(R["tx"]["stage"]["height"], height() + 1):
        br = requests.get(RPC + "/block_results?height=%d" % hgt, timeout=10).json()["result"]
        evs = (br.get("finalize_block_events") or []) + [e for t in (br.get("txs_results") or []) for e in t.get("events", [])]
        for e in evs:
            kv = {a["key"]: a["value"] for a in e.get("attributes", [])}
            if kv.get("_contract_address") == contract and "minted" in kv and kv["minted"] not in ("0",):
                paid_h = hgt; R["payout_event"] = {"height": hgt, "type": e["type"], "attrs": kv,
                    "in_tx": any(e is x for t in (br.get("txs_results") or []) for x in t.get("events", []))}
        if paid_h: break
    R["tx"]["sudo_payout"] = {"height": paid_h, "txhash": None, "path": "epoch sudo (finalize_block, no tx)" if paid_h and not R["payout_event"]["in_tx"] else "see payout_event"}
    d1, d2 = after["recipient1"] - before["recipient1"], after["recipient2"] - before["recipient2"]
    step("sudo_payout_exact", d1 == 1500000 and d2 == 2500000 and paid_h is not None,
         "delta r1 %d r2 %d payout height %s epoch_root height %s" % (d1, d2, paid_h, er.get("height")))
    res = tx("owner", 5000000, "wasm", "execute", contract, stage)
    R["negatives"]["double_payout_restage"] = "code %s minted %s %s" % (res.get("code"), attr(res, "minted"), res.get("raw_log", "")[:200])
    step("neg_double_payout_restage", (res.get("code") != 0 or attr(res, "minted") in ("0", None)) and bal(R1) == after["recipient1"],
         R["negatives"]["double_payout_restage"])
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
