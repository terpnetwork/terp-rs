#!/usr/local/bin/python3
"""Headstash vk_id 3 limits benchmark (loopback only, real proofs, local v6.4.0 chain).

Fresh single-node terpd v6.4.0 (/tmp/terpd-v640-build) on 127.0.0.1 with mainnet
consensus params (block max_gas 50,000,000, max_bytes 16,777,216, read once from
mainnet REST, read-only). Deploys the current cw-headstash wasm, fills the depth-17
note tree, and measures registration / claim / anchor / query costs. Test identities
and note secrets exist only in this process; filler key_coms are random field elements.
Wallet keys live in the bench chain's keyring-test and are never read or printed.
Writes public results to $BENCH_RUN/bench.json (checkpointed).

  bench_limits.py setup     # (re)create the bench chain home and start terpd
  bench_limits.py run       # all phases
Env: FILL_TARGET (default 131072 leaves), ANCHOR_TARGET (4400), PACK_CLAIMS (250),
     TIMEOUT_COMMIT (500ms), PROVERS (4).
"""
import base64, hashlib, json, os, random, secrets, subprocess, sys, threading, time
import concurrent.futures as cf
from pathlib import Path
import requests

HERE = os.path.dirname(os.path.abspath(__file__))
TERPD = os.environ.get("TERPD", "/tmp/terpd-v640-build/terpd")
H = Path(os.environ.get("BENCH_HOME", "/tmp/headstash-bench-chain"))
RUN = Path(os.environ.get("BENCH_RUN", str(H / "run")))
CID = "headstash-bench"
P = {"rpc": 45667, "p2p": 45666, "proxy": 45668, "pprof": 45061, "prom": 45670, "rest": 45327, "grpc": 45091}
NODE, RPC, REST, PROM = (f"tcp://127.0.0.1:{P['rpc']}", f"http://127.0.0.1:{P['rpc']}",
                         f"http://127.0.0.1:{P['rest']}", f"http://127.0.0.1:{P['prom']}/metrics")
RELAY_BIND = "127.0.0.1:18895"
WASM = os.environ.get("WASM", "/tmp/hsdirect/opt-out/cw_headstash.wasm")
WASM_SHA = "43a8e5d88a3af34dc09bd63f85f88a1b2b0a008a4b68ef45a83ed434894149a9"
ELIG_BIN = "/tmp/hm-build-target/release/headstash-eligibility"
RELAY_BIN = "/tmp/hm-build-target/release/headstash-register-relay"
EX = "/tmp/hm-build-target/release/examples"
WAVS_BIN = "/tmp/cwhs-native-target/release/examples/print_wavs"
MAINNET_MAX_GAS, MAINNET_MAX_BYTES = 50_000_000, 16_777_216
GP = 0.025
TIMEOUT_COMMIT = os.environ.get("TIMEOUT_COMMIT", "500ms")
DEPTH, CAP = 17, 1 << 17
FILL_TARGET = int(os.environ.get("FILL_TARGET", str(CAP)))
ANCHOR_TARGET = int(os.environ.get("ANCHOR_TARGET", "4400"))
PACK_CLAIMS = int(os.environ.get("PACK_CLAIMS", "250"))
PROVERS = int(os.environ.get("PROVERS", "4"))
CLAIM_IDS = int(os.environ.get("CLAIM_IDS", "4"))      # real-secret identities, 256 notes each
NICE = ["nice", "-n", "10"]
NODE_BIN = "/opt/homebrew/bin/node"
WASM_PROVER_DIR = os.path.expanduser("~/abstract/terp-core/websites/dao-dao-ui-headstash-direct/apps/dapp/public/headstash-prover/membership")
for u in (RPC, REST, PROM): assert u.startswith("http://127.0.0.1:")
assert str(H).startswith("/tmp/headstash-bench")

sys.path.insert(0, HERE)
os.environ.setdefault("RUN_DIR", str(RUN)); os.environ["CHAIN_HOME"] = str(H)
import run as b  # crypto + hashing helpers only (pmul, kec, eip191, evm_sign, list_hash, pallas_rand)
from run import pmul, kec, eip191, evm_sign, pallas_rand, list_hash, b64, h2b64, N, DENOMS, NOTE_VK_HASH, DENOM

CLAIM_GAS = int(os.environ.get("HEADSTASH_CLAIM_GAS", "300000"))
B = {"meta": {}, "reg": {}, "claims": {}, "fill": [], "anchors": [], "query": {}, "prove": {}, "blocks": {}, "relay": {}, "notes": []}
def log(*a): print(time.strftime("%H:%M:%S"), *a, flush=True)
def save():
    RUN.mkdir(parents=True, exist_ok=True)
    tmp = RUN / "bench.json.tmp"; tmp.write_text(json.dumps(B, indent=1)); tmp.replace(RUN / "bench.json")

# ------------------------------------------------------------------ chain
def terpd(*a, check=True, inp=None):
    p = subprocess.run([*NICE, TERPD, *a], capture_output=True, text=True, input=inp)
    if check and p.returncode: raise RuntimeError("terpd %s: %s" % (a[:3], (p.stderr or p.stdout)[-600:]))
    return p.stdout
def kr(*a): return terpd(*a, "--keyring-backend", "test", "--home", str(H))
def addr_of(name): return kr("keys", "show", name, "-a").strip()
def height(): return int(requests.get(RPC + "/status", timeout=10).json()["result"]["sync_info"]["latest_block_height"])

def setup():
    if (H / "terpd.pid").exists():
        subprocess.run(["kill", (H / "terpd.pid").read_text().strip()], capture_output=True); time.sleep(3)
    subprocess.run(["rm", "-rf", str(H)]); H.mkdir(parents=True); os.chmod(H, 0o700); os.umask(0o077)
    terpd("init", "bench", "--chain-id", CID, "--default-denom", "uterp", "--home", str(H))
    names = ["owner", "relayer", "stranger", "filler", "anchorer"]
    for n in names: kr("keys", "add", n, "--output", "json")   # mnemonics discarded
    bal = {"owner": "1000000000000000", "relayer": "100000000000000", "stranger": "100000000000000",
           "filler": "100000000000000", "anchorer": "100000000000000"}
    for n in names: terpd("genesis", "add-genesis-account", addr_of(n), bal[n] + "uterp", "--home", str(H), "--keyring-backend", "test")
    kr("genesis", "gentx", "owner", "1000000000uterp", "--chain-id", CID)
    terpd("genesis", "collect-gentxs", "--home", str(H))
    g = json.loads((H / "config/genesis.json").read_text())
    blk = g["consensus"]["params"]["block"]
    blk["max_gas"] = str(MAINNET_MAX_GAS); blk["max_bytes"] = str(MAINNET_MAX_BYTES)
    (H / "config/genesis.json").write_text(json.dumps(g, indent=1))
    c = (H / "config/config.toml").read_text()
    for o, n in [('tcp://127.0.0.1:26658', f'tcp://127.0.0.1:{P["proxy"]}'), ('tcp://127.0.0.1:26657', f'tcp://127.0.0.1:{P["rpc"]}'),
                 ('tcp://0.0.0.0:26656', f'tcp://127.0.0.1:{P["p2p"]}'), ('localhost:6060', f'127.0.0.1:{P["pprof"]}'),
                 ('":26660"', f'"127.0.0.1:{P["prom"]}"'), ('timeout_commit = "5s"', f'timeout_commit = "{TIMEOUT_COMMIT}"'),
                 ('timeout_commit = "1s"', f'timeout_commit = "{TIMEOUT_COMMIT}"'), ("prometheus = false", "prometheus = true")]:
        c = c.replace(o, n)
    (H / "config/config.toml").write_text(c)
    a = (H / "config/app.toml").read_text()
    for o, n in [('minimum-gas-prices = "0stake"', 'minimum-gas-prices = "0.025uterp"'), ('minimum-gas-prices = ""', 'minimum-gas-prices = "0.025uterp"'),
                 ('tcp://localhost:1317', f'tcp://127.0.0.1:{P["rest"]}'), ('localhost:9090', f'127.0.0.1:{P["grpc"]}')]:
        a = a.replace(o, n)
    i = a.index("[api]"); j = a.index("enable = ", i)
    if a.startswith("enable = false", j): a = a[:j] + "enable = true" + a[j + len("enable = false"):]
    (H / "config/app.toml").write_text(a)
    for f in ("config.toml", "app.toml"):
        for l in (H / "config" / f).read_text().splitlines():
            s = l.strip()
            if s and not s.startswith("#") and ("0.0.0.0" in s or "[::]" in s): raise SystemExit(f"public bind {f}: {s}")
    terpd("genesis", "validate", "--home", str(H))
    start()

def start():
    lf = open(H / "terpd.log", "a")
    p = subprocess.Popen([*NICE, TERPD, "start", "--home", str(H)], stdout=lf, stderr=subprocess.STDOUT)
    (H / "terpd.pid").write_text(str(p.pid))
    for _ in range(120):
        time.sleep(1)
        try:
            if height() >= 2: log("chain up pid", p.pid); return
        except Exception: pass
    raise SystemExit("chain did not start")

def stop():
    pid = (H / "terpd.pid").read_text().strip()
    subprocess.run(["kill", pid]); 
    for _ in range(60):
        if subprocess.run(["kill", "-0", pid], capture_output=True).returncode: return
        time.sleep(0.5)

def set_app(key, value):
    a = (H / "config/app.toml").read_text().splitlines()
    a = [f"{key} = {value}" if l.startswith(key + " = ") else l for l in a]
    (H / "config/app.toml").write_text("\n".join(a) + "\n")

def account(addr):
    r = requests.get(REST + "/cosmos/auth/v1beta1/accounts/" + addr, timeout=10).json()["account"]
    r = r.get("base_account", r)
    return int(r["account_number"]), int(r.get("sequence", 0))

def balance(addr):
    r = requests.get(REST + "/cosmos/bank/v1beta1/balances/%s/by_denom" % addr, params={"denom": "uterp"}, timeout=10).json()
    return int(r["balance"]["amount"])

def smart(contract, msg):
    t0 = time.time()
    r = requests.get(REST + "/cosmwasm/wasm/v1/contract/%s/smart/%s" % (contract, b64(json.dumps(msg).encode())), timeout=60)
    dt = time.time() - t0
    j = r.json()
    if "data" not in j: raise RuntimeError("query: " + json.dumps(j)[:300])
    return j["data"], dt

def cli_tx(frm, gas, *args, amount=None):
    a = ["tx", *args, "--from", frm, "--home", str(H), "--keyring-backend", "test", "--chain-id", CID, "--node", NODE,
         "--gas", str(gas), "--fees", "%duterp" % fee(gas), "-b", "sync", "-y", "-o", "json"]
    if amount: a += ["--amount", amount]
    p = subprocess.run([*NICE, TERPD, *a], capture_output=True, text=True)
    try: res = json.loads(p.stdout)
    except Exception: return {"code": -1, "raw_log": (p.stdout + p.stderr)[-600:]}
    if res.get("code", 0) != 0: return res
    return wait_tx(res["txhash"])

def fee(gas): return int(gas * GP + 0.999999)

def wait_tx(h, t=120):
    end = time.time() + t
    while time.time() < end:
        r = requests.get(RPC + "/tx", params={"hash": "0x" + h}, timeout=10).json()
        if "result" in r:
            x = r["result"]; tr = x["tx_result"]
            return {"txhash": h, "height": int(x["height"]), "code": tr.get("code", 0), "gas_used": int(tr.get("gas_used", 0)),
                    "gas_wanted": int(tr.get("gas_wanted", 0)), "raw_log": tr.get("log", ""), "events": tr.get("events", []),
                    "tx_bytes": len(base64.b64decode(x["tx"]))}
        time.sleep(0.25)
    raise RuntimeError("tx not found " + h)

def attr(res, key):
    for ev in res.get("events", []):
        for at in ev.get("attributes", []):
            if at.get("key") == key: return at.get("value")

# offline sign + encode + broadcast_tx_sync (multi-message txs)
SIGN_DIR = RUN / "sign"
def sign_tx(frm_name, frm_addr, acct, seq, msgs, gas, granter=""):
    SIGN_DIR.mkdir(parents=True, exist_ok=True)
    body = {"body": {"messages": msgs, "memo": "", "timeout_height": "0", "extension_options": [], "non_critical_extension_options": []},
            "auth_info": {"signer_infos": [], "fee": {"amount": [{"denom": "uterp", "amount": str(fee(gas))}], "gas_limit": str(gas),
                                                       "payer": "", "granter": granter}, "tip": None}, "signatures": []}
    f = SIGN_DIR / ("%s-%d-%s.json" % (frm_name, seq, secrets.token_hex(4)))
    f.write_text(json.dumps(body))
    signed = terpd("tx", "sign", str(f), "--from", frm_name, "--offline", "--account-number", str(acct), "--sequence", str(seq),
                   "--chain-id", CID, "--keyring-backend", "test", "--home", str(H), "--sign-mode", "direct", "-o", "json")
    f.write_text(signed)
    enc = terpd("tx", "encode", str(f)).strip()
    f.unlink()
    return enc

def broadcast(enc):
    r = requests.post(RPC, json={"jsonrpc": "2.0", "id": 1, "method": "broadcast_tx_sync", "params": {"tx": enc}}, timeout=60).json()
    if "result" not in r: return {"code": -1, "raw_log": json.dumps(r)[:300]}
    x = r["result"]
    return {"code": int(x.get("code", 0)), "txhash": x.get("hash"), "raw_log": x.get("log", ""), "bytes": len(base64.b64decode(enc))}

def exe(sender, contract, msg, funds=None):
    return {"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": sender, "contract": contract, "msg": msg, "funds": funds or []}

class Acct:
    def __init__(self, name):
        self.name, self.addr = name, addr_of(name)
        self.num, self.seq = account(self.addr)
        self.lock = threading.Lock()
    def resync(self): self.num, self.seq = account(self.addr)
    def send(self, msgs, gas, granter="", wait=True):
        with self.lock:
            enc = sign_tx(self.name, self.addr, self.num, self.seq, msgs, gas, granter)
            r = broadcast(enc)
            if r["code"] == 0: self.seq += 1
            elif "sequence" in r["raw_log"]:
                self.resync(); enc = sign_tx(self.name, self.addr, self.num, self.seq, msgs, gas, granter); r = broadcast(enc)
                if r["code"] == 0: self.seq += 1
        if r["code"] != 0 or not wait: return r
        out = wait_tx(r["txhash"]); out["tx_bytes"] = r["bytes"]; return out

# ---------------------------------------------------------- finalize_block sampler
FB = {}  # block height -> finalize_block seconds
def sampler(stop_ev):
    last = None
    while not stop_ev.is_set():
        try:
            s = c = None
            for ln in requests.get(PROM, timeout=2).text.splitlines():
                if 'method="finalize_block"' in ln:
                    if ln.startswith("cometbft_abci_connection_method_timing_seconds_sum"): s = float(ln.split()[-1])
                    elif ln.startswith("cometbft_abci_connection_method_timing_seconds_count"): c = int(float(ln.split()[-1]))
            if s is not None and c is not None:
                if last and c == last[1] + 1:
                    FB[c] = s - last[0]   # keyed by finalize count; mapped to heights via FB_OFFSET
                last = (s, c)
        except Exception: pass
        time.sleep(0.05)
FB_OFFSET = [0]
def calib():
    for ln in requests.get(PROM, timeout=2).text.splitlines():
        if ln.startswith("cometbft_abci_connection_method_timing_seconds_count") and 'method="finalize_block"' in ln:
            c = int(float(ln.split()[-1])); h = height()
            FB_OFFSET[0] = h - c
            return
def fb_ms(h):
    v = FB.get(h - FB_OFFSET[0])
    return round(v * 1000, 1) if v is not None else None

def block_summary(h):
    r = requests.get(RPC + "/block_results", params={"height": h}, timeout=20).json()["result"]
    blk = requests.get(RPC + "/block", params={"height": h}, timeout=20).json()["result"]["block"]
    txs = r.get("txs_results") or []
    raw = blk["data"].get("txs") or []
    return {"height": h, "txs": len(txs), "ok": sum(1 for t in txs if int(t.get("code", 0)) == 0),
            "gas_used": sum(int(t.get("gas_used", 0)) for t in txs), "gas_wanted": sum(int(t.get("gas_wanted", 0)) for t in txs),
            "tx_bytes": sum(len(base64.b64decode(x)) for x in raw), "finalize_ms": fb_ms(h), "time": blk["header"]["time"]}

# ------------------------------------------------------------------ identities
CTX_TAG = b"terp-hs-direct-ctx-v1"
def direct_ctx(contract):
    h = bytearray(hashlib.sha256(CTX_TAG + b"\0" + CID.encode() + b"\0" + contract.encode()).digest()); h[31] &= 0x3F
    return bytes(h)

def key_com(secret):
    return subprocess.run([*NICE, EX + "/membership_key_com"], input=secret.hex(), capture_output=True, text=True, check=True).stdout.strip()

class Ident:
    def __init__(self, real):
        self.priv = secrets.randbelow(N - 1) + 1
        pt = pmul(self.priv)
        self.addr = kec(pt[0].to_bytes(32, "big") + pt[1].to_bytes(32, "big"))[12:]
        self.real = real
        self.next = 0
        self.notes = []; self.secrets = []; self.kcs = []
    def ident_str(self): return self.addr.hex()

def main_run():
    t_start = time.time()
    B["meta"] = {"started": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "terpd": terpd("version").strip(), "terpd_path": TERPD,
                 "chain_id": CID, "timeout_commit": TIMEOUT_COMMIT, "max_gas": MAINNET_MAX_GAS, "max_bytes": MAINNET_MAX_BYTES,
                 "gas_price": "0.025uterp", "host": subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True).stdout.strip(),
                 "fill_target": FILL_TARGET, "anchor_target": ANCHOR_TARGET}
    g = requests.get(REST + "/cosmos/consensus/v1/params", timeout=10).json()["params"]["block"]
    B["meta"]["local_block_params"] = g
    assert int(g["max_gas"]) == MAINNET_MAX_GAS and int(g["max_bytes"]) == MAINNET_MAX_BYTES, g
    wsha = hashlib.sha256(open(WASM, "rb").read()).hexdigest(); assert wsha == WASM_SHA, wsha
    B["meta"]["wasm_sha256"] = wsha
    stop_ev = threading.Event(); threading.Thread(target=sampler, args=(stop_ev,), daemon=True).start()
    time.sleep(1); calib()
    owner, relayer, stranger, filler, anchorer = (Acct(n) for n in ("owner", "relayer", "stranger", "filler", "anchorer"))

    # ---- identities + eligibility tree
    n_ids = (FILL_TARGET + 255) // 256
    ids = [Ident(i < CLAIM_IDS) for i in range(n_ids)]
    comm = RUN / "communities" / "bench"; comm.mkdir(parents=True, exist_ok=True)
    lines = ["addr,amount"]
    left = FILL_TARGET
    for idn in ids:
        k = min(256, left); left -= k
        lines.append("0x%s,%d" % (idn.addr.hex(), 5000 * k))   # k notes of class 11 (5,000 TERP)
    (comm / "bench.csv").write_text("\n".join(lines) + "\n")
    elig = RUN / "eligibility"
    t0 = time.time()
    subprocess.run([*NICE, ELIG_BIN, str(RUN / "communities"), str(elig), "--decimals", "bench=6", "--denominations"], check=True, capture_output=True)
    ntree = json.loads((elig / "notes" / "tree.json").read_text())
    by = {}
    with open(elig / "notes" / "note_paths.jsonl") as f:
        for l in f:
            p = json.loads(l); by.setdefault(p["identity"], []).append(p)
    for idn in ids:
        idn.notes = sorted(by[idn.ident_str()], key=lambda p: p["note_index"])
    total = sum(len(i.notes) for i in ids)
    B["meta"]["eligibility"] = {"identities": n_ids, "notes": total, "root": ntree["root_hex"], "depth": ntree["depth"],
                                "secs": round(time.time() - t0, 1), "classes": sorted({p["class"] for i in ids for p in i.notes})}
    log("eligibility", B["meta"]["eligibility"]); save()
    # key commitments: real for claim identities, random field elements for fillers
    t0 = time.time()
    with cf.ThreadPoolExecutor(6) as ex:
        for idn in ids:
            if idn.real:
                idn.secrets = [pallas_rand() for _ in idn.notes]
                idn.kcs = list(ex.map(key_com, idn.secrets))
            else:
                idn.kcs = [pallas_rand().hex() for _ in idn.notes]
    B["meta"]["key_com_secs"] = round(time.time() - t0, 1)
    log("key_coms done", B["meta"]["key_com_secs"])

    # ---- circuit + contract
    cdir = RUN / "circuit"; cdir.mkdir(parents=True, exist_ok=True)
    subprocess.run([*NICE, EX + "/bench_note_proofs", str(cdir), "0"], check=True, capture_output=True)
    assert hashlib.sha256((cdir / "blob.bin").read_bytes()).hexdigest() == NOTE_VK_HASH
    res = cli_tx("owner", 15_000_000, "wasm", "store-full-circuit", str(cdir / "params.bin"), str(cdir / "vk_body.bin"),
                 "--k", "11", "--circuit-type", "0", "--curve-type", "0")
    assert res.get("code") == 0, res.get("raw_log")
    zk_id = None
    for ev in res["events"]:
        if "circuit" in ev["type"]:
            for at in ev["attributes"]:
                if at["key"] in ("zk_id", "circuit_id", "zkid", "id") and str(at["value"]).isdigit() and zk_id is None: zk_id = int(at["value"])
    zk_id = zk_id or 1
    B["gas_setup"] = {"store_circuit": res["gas_used"]}
    res = cli_tx("owner", 20_000_000, "wasm", "store", WASM); assert res.get("code") == 0, res.get("raw_log")
    code_id = attr(res, "code_id"); B["gas_setup"]["store_wasm"] = res["gas_used"]
    wavs = json.loads(subprocess.run([WAVS_BIN], capture_output=True, text=True, check=True).stdout)
    init = {"genesis_root": b64(bytes(32)), "genesis_label": "headstash-bench",
            "token_strategy": {"existing_fungible": {"proof": h2b64(DENOM), "raw": "uterp"}}, "wavs": wavs}
    claim_notes = CLAIM_IDS * 256
    fund = claim_notes * DENOMS[11] + 10_000_000
    res = cli_tx("owner", 5_000_000, "wasm", "instantiate", code_id, json.dumps(init), "--label", "headstash-bench",
                 "--admin", owner.addr, amount="%duterp" % fund)
    assert res.get("code") == 0, res.get("raw_log")
    contract = attr(res, "_contract_address"); B["meta"]["contract"] = contract; B["gas_setup"]["instantiate"] = res["gas_used"]
    ctx = direct_ctx(contract)
    classes = {}
    for idn in ids:
        for p in idn.notes: classes[p["class"]] = classes.get(p["class"], 0) + 1
    camp = {"set_epoch_campaign": {"campaign": {
        "ctx": b64(ctx), "backend": "halo2", "cutoff_height": height() + 10_000_000, "bech32_hrp": "terp", "denom": h2b64(DENOM),
        "amounts": [{"amount": DENOMS[c], "cap": n} for c, n in sorted(classes.items())], "reserved": [], "trusted": [],
        "trusted_verify_threshold": 0, "trusted_sign_threshold": 0, "max_rows": 8, "allocation_root": b64(bytes(32)), "root_id": 0,
        "vk_id": 3, "vk_hash": h2b64(NOTE_VK_HASH), "sweep_recipient": "",
        "registration": {"eligibility_root": h2b64(ntree["root_hex"]), "eligibility_depth": 17, "min_anonymity_set": 2,
                         "anchor_start_height": 1, "anchor_interval": 1}}}}
    res = cli_tx("owner", 5_000_000, "wasm", "execute", contract, json.dumps(camp))
    if res.get("code") != 0:
        cc = camp["set_epoch_campaign"]["campaign"]; cc["trusted"] = [b64(bytes(32))]; cc["trusted_verify_threshold"] = 1; cc["trusted_sign_threshold"] = 1
        res = cli_tx("owner", 5_000_000, "wasm", "execute", contract, json.dumps(camp))
    assert res.get("code") == 0, res.get("raw_log")
    res = cli_tx("owner", 3_000_000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id}}))
    assert res.get("code") == 0, res.get("raw_log"); B["gas_setup"]["bind_circuit"] = res["gas_used"]
    log("contract", contract, "zk_id", zk_id); save()

    reward = owner.addr
    def wire(idn): return [{"class": p["class"], "key_com": h2b64(idn.kcs[i])} for i, p in enumerate(idn.notes)]
    WIRE = {}
    def reg_msg(idn, start, n):
        if id(idn) not in WIRE: WIRE[id(idn)] = wire(idn)
        lst = WIRE[id(idn)]
        sig = None
        if start == 0:
            lh = list_hash(ctx, 0x01, idn.addr, [(p["class"], idn.kcs[i]) for i, p in enumerate(idn.notes)]).hex()
            sig = b64(evm_sign(idn.priv, eip191("headstash-register-notes-v3:%s:%s:%d:%s" % (ctx.hex(), lh, len(lst), reward))))
        rows = idn.notes[start:start + n]
        return {"register_notes": {"signer": {"evm": {"address": b64(idn.addr)}}, "signature": sig, "list": lst, "start": start,
                "eligibility": [{"index": r["leaf_index"], "path": [h2b64(x) for x in r["path"]]} for r in rows],
                "reward_addr": reward if start == 0 else None}}
    def take(idn, n):
        m = reg_msg(idn, idn.next, n); idn.next += n; return m
    leaves = [0]
    def tree_info(): return smart(contract, {"registration_tree": {}})[0]

    fillers = [i for i in ids if not i.real]
    claimers = [i for i in ids if i.real]

    # ---- 1. registration chunk gas 1..32: first chunk (signature + N6 over a 256-entry list) vs continuation chunk
    reg = B["reg"]; reg["first_chunk"] = {}; reg["cont_chunk"] = {}
    for n in range(1, 33):
        idn = fillers[n - 1]
        r1 = filler.send([exe(filler.addr, contract, take(idn, n))], 12_000_000)
        r2 = filler.send([exe(filler.addr, contract, take(idn, n))], 12_000_000)
        assert r1["code"] == 0 and r2["code"] == 0, (r1.get("raw_log"), r2.get("raw_log"))
        reg["first_chunk"][n] = {"gas": r1["gas_used"], "tx_bytes": r1["tx_bytes"]}
        reg["cont_chunk"][n] = {"gas": r2["gas_used"], "tx_bytes": r2["tx_bytes"]}
        leaves[0] += 2 * n
        log("chunk", n, r1["gas_used"], r2["gas_used"])
    save()
    # signature-free first chunk cost cannot be isolated on chain (start 0 needs a signature);
    # difference first - cont at equal n = signature verify + N6 list scan + list-state init.
    # ---- 2. notes per tx: K continuation chunks of 32 in one tx
    reg["multi_chunk_tx"] = {}
    pool = [i for i in fillers[32:] ]
    def full_chunk_src():
        for idn in pool:
            while idn.next + 32 <= len(idn.notes): yield idn
    src = full_chunk_src()
    # first chunks for the pool identities happen lazily inside fill; here use identities already started
    for K in (1, 2, 3, 4, 5):
        msgs, snap = [], []
        for _ in range(K):
            idn = next(src); snap.append((idn, idn.next)); msgs.append(exe(filler.addr, contract, take(idn, 32)))
        gas = min(MAINNET_MAX_GAS, K * 11_000_000)
        r = filler.send(msgs, gas)
        reg["multi_chunk_tx"][K] = {"code": r["code"], "gas_used": r.get("gas_used"), "gas_wanted": gas, "tx_bytes": r.get("tx_bytes"),
                                    "log": r.get("raw_log", "")[:200]}
        if r["code"] == 0: leaves[0] += 32 * K
        else:
            for idn, st in reversed(snap): idn.next = st
        log("multi", K, reg["multi_chunk_tx"][K])
        if r["code"] != 0: break
    save()

    # ---- 3. claim identities fully registered (real secrets)
    for idn in claimers:
        while idn.next < len(idn.notes):
            r = filler.send([exe(filler.addr, contract, take(idn, 32))], 12_000_000); assert r["code"] == 0, r.get("raw_log")
            leaves[0] += 32
    log("claimers registered, leaves", leaves[0])

    # ---- prover
    prove_log = B["prove"].setdefault("native", [])
    claimable = [(idn, i) for idn in claimers for i in range(len(idn.notes))]
    random.shuffle(claimable)
    def prove(note, dest, root):
        idn, i = note
        pth, qdt = smart(contract, {"registration_path": {"key_com": h2b64(idn.kcs[i]), "anchor": b64(root)}})
        wit = json.dumps({"secret_hex": idn.secrets[i].hex(), "ctx_hex": ctx.hex(), "root_hex": root.hex(), "destination": dest,
                          "amount": str(pth["amount"]), "index": pth["index"], "siblings_hex": pth["siblings"]})
        t0 = time.time()
        out = subprocess.run([*NICE, EX + "/prove_note_witness", "/dev/stdin"], input=wit, capture_output=True, text=True)
        dt = time.time() - t0
        if out.returncode != 0: raise RuntimeError(out.stderr[-200:])
        pr = json.loads(out.stdout); f = pr["body"].split(":")
        prove_log.append({"secs": round(dt, 3), "path_query_ms": round(qdt * 1000, 1)})
        if len(B["prove"].setdefault("wasm_node", [])) < 3:
            w_ = subprocess.run([*NICE, NODE_BIN, os.path.join(HERE, "bench_wasm_prove.mjs"), WASM_PROVER_DIR], input=wit, capture_output=True, text=True)
            try: B["prove"]["wasm_node"].append(json.loads(w_.stdout.strip().splitlines()[-1]))
            except Exception: B["prove"]["wasm_node"].append({"error": (w_.stderr or w_.stdout)[-200:]})
        return {"root": b64(root), "nullifier": h2b64(f[3]), "amount": int(f[5]), "proof": h2b64(pr["proof_hex"]), "dest": f[4],
                "proof_bytes": pr["proof_bytes"]}
    def claim_msg(pr): return {"claim_note": {k: pr[k] for k in ("root", "nullifier", "amount", "proof")}}

    # ---- anchors
    def accept(wait=True):
        rt = tree_info()
        while height() < int(rt.get("next_anchor_height", 0)): time.sleep(0.1)
        return anchorer.send([exe(anchorer.addr, contract, {"accept_anchor": {}})], ACCEPT_GAS[0], wait=wait)
    ACCEPT_GAS = [3_000_000]
    def latest_anchor():
        rt = tree_info(); return base64.b64decode(rt["latest_anchor"]["root"]), rt["anchor_count"], rt
    wallet_n = [0]
    def fresh_wallet():
        wallet_n[0] += 1; name = "bw%05d" % wallet_n[0]
        terpd("keys", "delete", name, "-y", "--keyring-backend", "test", "--home", str(H), check=False)
        kr("keys", "add", name, "--output", "json")
        return name, addr_of(name)

    def checkpoint_claims(label, k=2):
        """k single claims from fee-granted fresh wallets against the latest anchor (worst-case scan position) and the oldest."""
        root, na, rt = latest_anchor()
        out = []
        for j in range(k):
            name, w = fresh_wallet()
            r_old = base64.b64decode(smart(contract, {"registration_anchors": {"limit": 1}})[0]["anchors"][0]["root"]) if j == 1 and rt["anchor_count"] > 1 else root
            note = claimable.pop()
            try: pr = prove(note, w, r_old)
            except RuntimeError as e:  # note not under the oldest anchor: use the latest
                r_old = root; pr = prove(note, w, root)
            g = relayer.send([feegrant_msg(w, CLAIM_GAS)], 200_000)
            assert g["code"] == 0, g.get("raw_log")
            a = Acct(name)
            r = a.send([exe(w, contract, claim_msg(pr))], CLAIM_GAS, granter=relayer.addr)
            oog = None
            if r["code"] == 11:   # out of gas at the production CLAIM_GAS: re-grant and retry with headroom to measure
                oog = r.get("gas_used")
                g2 = relayer.send(list(feegrant_msg_update(w, 20_000_000)), 300_000)
                r = a.send([exe(w, contract, claim_msg(pr))], 20_000_000, granter=relayer.addr)
            out.append({"out_of_gas_at_400k": oog, "anchor_pos": "oldest" if r_old != root else "latest", "gas": r.get("gas_used"), "code": r["code"],
                        "log": r.get("raw_log", "")[:160], "tx_bytes": r.get("tx_bytes"), "grant_gas": g["gas_used"]})
        # path query latency for a claim note at the latest anchor, and the registration_tree query
        idn, i = claimable[-1]
        lat = []
        for _ in range(5):
            _, dt = smart(contract, {"registration_path": {"key_com": h2b64(idn.kcs[i]), "anchor": b64(root)}}); lat.append(dt * 1000)
        _, dtt = smart(contract, {"registration_tree": {}})
        c = {"label": label, "leaves": rt["leaf_count"] if "leaf_count" in rt else leaves[0], "anchors": na, "claims": out,
             "path_query_ms": {"min": round(min(lat), 1), "median": round(sorted(lat)[2], 1), "max": round(max(lat), 1)},
             "registration_tree_query_ms": round(dtt * 1000, 1), "height": height()}
        B["claims"].setdefault("checkpoints", []).append(c); save()
        log("checkpoint", label, json.dumps(c)[:300])
        return c

    def feegrant_msg(grantee, gas):
        exp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() + 3600 * 24))
        return {"@type": "/cosmos.feegrant.v1beta1.MsgGrantAllowance", "granter": relayer.addr, "grantee": grantee,
                "allowance": {"@type": "/cosmos.feegrant.v1beta1.AllowedMsgAllowance",
                              "allowance": {"@type": "/cosmos.feegrant.v1beta1.BasicAllowance",
                                            "spend_limit": [{"denom": "uterp", "amount": str(fee(gas))}], "expiration": exp},
                              "allowed_messages": ["/cosmwasm.wasm.v1.MsgExecuteContract"]}}

    def feegrant_msg_update(grantee, gas):
        return {"@type": "/cosmos.feegrant.v1beta1.MsgRevokeAllowance", "granter": relayer.addr, "grantee": grantee}, feegrant_msg(grantee, gas)
    def anchor_rec(r, na):
        B["anchors"].append({"anchors_after": na, "gas": r.get("gas_used"), "code": r.get("code"), "height": r.get("height")})

    # anchors 1..10
    while True:
        r = accept(); root, na, _ = latest_anchor(); anchor_rec(r, na)
        if r["code"] != 0: log("accept fail", r.get("raw_log")); 
        if na >= 10: break
        # need a root change between anchors: one continuation note
        idn = next(i for i in fillers if 0 < i.next < len(i.notes))
        filler.send([exe(filler.addr, contract, take(idn, 1))], 2_000_000); leaves[0] += 1
    checkpoint_claims("anchors_10", 3)

    # ---- 4/5. block loop: per block one presigned accept_anchor tx (anchorer) + one presigned registration tx (filler)
    def presign(acct, items, granter=""):
        seq0 = acct.seq
        with cf.ThreadPoolExecutor(4) as ex:
            return list(ex.map(lambda k: sign_tx(acct.name, acct.addr, acct.num, seq0 + k, items[k][0], items[k][1], granter), range(len(items))))
    def block_loop(builder, done, phase, batch=25):
        last_h = height()
        while not done():
            items = []
            for _ in range(batch):
                it = builder()
                if it is None: break
                items.append(it)
            if not items: return
            fe = presign(filler, [(m, g) for m, g, _ in items])
            ae = presign(anchorer, [([exe(anchorer.addr, contract, {"accept_anchor": {}})], ACCEPT_GAS[0])] * len(items))
            sent = []
            for k, (msgs, gas, meta) in enumerate(items):
                while height() == last_h: time.sleep(0.03)
                last_h = height()
                ra = broadcast(ae[k]); rf = broadcast(fe[k])
                if ra["code"] == 0: anchorer.seq += 1
                if rf["code"] == 0: filler.seq += 1
                sent.append((ra, rf, msgs, gas, meta))
                if ra["code"] != 0 or rf["code"] != 0:
                    log("checktx reject", ra["raw_log"][:120], rf["raw_log"][:120]); break
            reset = {}
            for ra, rf, msgs, gas, meta in sent:
                rax = wait_tx(ra["txhash"]) if ra["code"] == 0 else ra
                rfx = wait_tx(rf["txhash"]) if rf["code"] == 0 else rf
                notes = sum(n for _, _, n in meta)
                if rfx["code"] == 0: leaves[0] += notes
                else:
                    for idn, st, n in meta: reset[id(idn)] = (idn, min(st, reset.get(id(idn), (idn, 1 << 30))[1]))
                if rax.get("code") == 0 and rax.get("gas_used"):
                    ACCEPT_GAS[0] = max(3_000_000, int(rax["gas_used"] * 1.3) + 300_000)
                B["anchors"].append({"phase": phase, "gas": rax.get("gas_used"), "code": rax.get("code"), "height": rax.get("height"),
                                     "log": "" if rax.get("code") == 0 else rax.get("raw_log", "")[:120]})
                B["fill"].append({"phase": phase, "height": rfx.get("height"), "notes": notes, "chunks": len(msgs), "gas_used": rfx.get("gas_used"),
                                  "gas_wanted": gas, "tx_bytes": rf.get("bytes"), "code": rfx.get("code"), "leaves": leaves[0],
                                  "first_chunks": sum(1 for _, st, _ in meta if st == 0), "finalize_ms": None,
                                  "log": "" if rfx.get("code") == 0 else rfx.get("raw_log", "")[:160]})
            for idn, st in reset.values(): idn.next = st
            if any(ra["code"] or rf["code"] for ra, rf, *_ in sent): filler.resync(); anchorer.resync()
            time.sleep(1.5)
            for f in B["fill"][-3 * batch:]:
                if f["finalize_ms"] is None and f["height"]: f["finalize_ms"] = fb_ms(f["height"])
            na = tree_info()["anchor_count"]
            NA[0] = na
            log(phase, "leaves", leaves[0], "anchors", na, "fill gas", B["fill"][-1]["gas_used"], "accept gas", B["anchors"][-1]["gas"],
                "fb_ms", B["fill"][-2]["finalize_ms"] if len(B["fill"]) > 1 else None)
            save()
            while anchor_marks and na >= anchor_marks[0]:
                checkpoint_claims("anchors_%d" % anchor_marks.pop(0), 3); last_h = height()
            while leaf_marks and leaves[0] >= leaf_marks[0]:
                checkpoint_claims("leaves_%d" % leaf_marks.pop(0), 2); last_h = height()
    NA = [0]
    fill_target_leaves = FILL_TARGET - ANCHOR_TARGET  # leave notes for one-note-per-block anchor growth
    anchor_marks = [m for m in (100, 1000) if m < ANCHOR_TARGET]
    leaf_marks = [m for m in (5_000, 10_000, 20_000, 30_000, 50_000, 75_000, 100_000, 120_000) if m < fill_target_leaves]
    reserve_ids, need = [], FILL_TARGET - fill_target_leaves
    for idn in reversed(fillers):
        if need <= 0: break
        reserve_ids.append(idn); need -= len(idn.notes) - idn.next
    fill_ids = [i for i in fillers if i not in reserve_ids]
    def fill_builder():
        msgs, gas, meta = [], 0, []
        room = MAINNET_MAX_GAS - ACCEPT_GAS[0] - 500_000
        planned = sum(i.next for i in ids)
        for idn in fill_ids:
            while idn.next < len(idn.notes) and gas + 11_000_000 <= room and planned < fill_target_leaves:
                n = min(32, len(idn.notes) - idn.next, fill_target_leaves - planned); planned += n
                meta.append((idn, idn.next, n)); msgs.append(exe(filler.addr, contract, take(idn, n))); gas += 11_000_000
            if gas + 11_000_000 > room or planned >= fill_target_leaves: break
        return (msgs, gas, meta) if msgs else None
    PENDING = []
    t_fill = time.time()
    block_loop(fill_builder, lambda: leaves[0] >= fill_target_leaves, "fill", batch=20)
    for f in B["fill"]:
        if f["finalize_ms"] is None and f["height"]: f["finalize_ms"] = fb_ms(f["height"])
    B["meta"]["fill_secs"] = round(time.time() - t_fill, 1); save()
    log("fill done", leaves[0], "in", B["meta"]["fill_secs"])
    def anchor_builder():
        for idn in reserve_ids:
            if idn.next < len(idn.notes):
                n = 1 if idn.next > 0 else min(32, len(idn.notes))
                meta = [(idn, idn.next, n)]
                return ([exe(filler.addr, contract, take(idn, n))], 12_000_000 if n > 1 else 2_500_000, meta)
        return None
    t_anc = time.time()
    block_loop(anchor_builder, lambda: NA[0] >= ANCHOR_TARGET, "anchor", batch=50)
    B["meta"]["anchor_phase_secs"] = round(time.time() - t_anc, 1)
    na = tree_info()["anchor_count"]
    checkpoint_claims("anchors_%d_final" % na, 3)
    B["meta"]["final_leaves"] = leaves[0]; B["meta"]["final_anchors"] = na; save()

    # ---- 6. claims per block: PACK_CLAIMS fee-granted fresh wallets, one claim tx each, broadcast together
    root, na, rt = latest_anchor()
    wallets = [fresh_wallet() for _ in range(PACK_CLAIMS)]
    notes = [claimable.pop() for _ in range(PACK_CLAIMS)]
    t0 = time.time()
    with cf.ThreadPoolExecutor(PROVERS) as ex:
        prs = list(ex.map(lambda x: prove(x[0], x[1][1], root), zip(notes, wallets)))
    B["prove"]["pack_wall_secs"] = round(time.time() - t0, 1); B["prove"]["pack_parallel"] = PROVERS
    # relay grants (latency, serial and concurrent) for the first 30 wallets
    env = dict(os.environ, HEADSTASH_RELAY_HOME=str(H), HEADSTASH_RELAY_FROM="relayer", HEADSTASH_KEYRING_BACKEND="test",
               HEADSTASH_REST=REST, HEADSTASH_CONTRACT=contract, HEADSTASH_CHAIN_ID=CID, HEADSTASH_RELAY_BIND=RELAY_BIND,
               HEADSTASH_GAS_PRICE="0.025uterp", HEADSTASH_CLAIM_GAS=str(CLAIM_GAS), HEADSTASH_REGISTER_GAS="12000000",
               HEADSTASH_GRANTS_PER_MIN="100000", HEADSTASH_FEEGRANT="1", TERPD=TERPD, PATH="/usr/local/bin:" + os.environ.get("PATH", ""))
    env.pop("HEADSTASH_ELIGIBILITY", None)
    rlog = open(RUN / "relay.log", "w")
    relay = subprocess.Popen([*NICE, RELAY_BIN], env=env, stdout=rlog, stderr=subprocess.STDOUT)
    granted = set()
    try:
        for _ in range(60):
            time.sleep(1)
            if "relay_up" in (RUN / "relay.log").read_text(): break
        relayer.resync()
        def rgrant(k):
            t0 = time.time()
            r = requests.post("http://" + RELAY_BIND + "/headstash/feegrant", json={"grantee": wallets[k][1], "claim": claim_msg(prs[k])["claim_note"]},
                              headers={"x-real-ip": "10.0.%d.%d" % (k // 250, k % 250)}, timeout=300)
            return k, r.status_code, round(time.time() - t0, 2), r.text[:160]
        ser = [rgrant(k) for k in range(min(10, PACK_CLAIMS))]
        with cf.ThreadPoolExecutor(20) as ex: con = list(ex.map(rgrant, range(10, min(30, PACK_CLAIMS))))
        B["relay"] = {"serial": [{"status": s, "secs": d} for _, s, d, _ in ser],
                      "concurrent20": [{"status": s, "secs": d, "body": t if s != 200 else ""} for _, s, d, t in con]}
        granted = {k for k, s, _, _ in ser + con if s == 200}
    finally:
        relay.terminate(); relay.wait(10)
    relayer.resync(); save()
    cg = [c["gas"] for cp in B["claims"]["checkpoints"] for c in cp["claims"] if c.get("gas") and c["code"] == 0]
    PACK_GAS = max(CLAIM_GAS, int(max(cg[-3:]) * 1.15)) if cg else CLAIM_GAS
    B["blocks"]["pack_gas_limit"] = PACK_GAS
    if PACK_GAS > CLAIM_GAS:  # relay grants were sized for CLAIM_GAS: replace them
        for k in sorted(granted): relayer.send(list(feegrant_msg_update(wallets[k][1], PACK_GAS)), 300_000)
    rest = [k for k in range(PACK_CLAIMS) if k not in granted]
    for i in range(0, len(rest), 100):   # batched direct grants (same allowance shape as the relay)
        part = rest[i:i + 100]
        r = relayer.send([feegrant_msg(wallets[k][1], PACK_GAS) for k in part], 60_000 + 40_000 * len(part))
        B["blocks"].setdefault("grant_batches", []).append({"grants": len(part), "gas": r.get("gas_used"), "code": r["code"]})
        assert r["code"] == 0, r.get("raw_log")
    # sign all claim txs, then broadcast them back to back
    accts = [Acct(n) for n, _ in wallets]
    encs = []
    with cf.ThreadPoolExecutor(4) as ex:
        encs = list(ex.map(lambda k: sign_tx(accts[k].name, accts[k].addr, accts[k].num, accts[k].seq,
                                             [exe(accts[k].addr, contract, claim_msg(prs[k]))], PACK_GAS, relayer.addr), range(PACK_CLAIMS)))
    h0 = height(); t0 = time.time()
    hs = [broadcast(e) for e in encs]
    B["blocks"]["broadcast_secs"] = round(time.time() - t0, 2)
    bad = [h for h in hs if h["code"] != 0]
    res = [wait_tx(h["txhash"]) for h in hs if h["code"] == 0]
    hh = sorted({r["height"] for r in res})
    time.sleep(2)
    B["blocks"]["pack"] = {"claims": PACK_CLAIMS, "checktx_rejects": len(bad), "checktx_reject_sample": [b_["raw_log"][:160] for b_ in bad[:3]],
                           "ok": sum(1 for r in res if r["code"] == 0), "claim_gas": sorted(r["gas_used"] for r in res if r["code"] == 0),
                           "claim_tx_bytes": sorted({r["tx_bytes"] for r in res}),
                           "blocks": [block_summary(h) for h in range(hh[0], hh[-1] + 1)] if hh else []}
    B["claims"]["pack_fail_logs"] = [r["raw_log"][:160] for r in res if r["code"] != 0][:5]
    save(); log("pack", json.dumps({k: v for k, v in B["blocks"]["pack"].items() if k != "claim_gas"})[:600])

    if os.environ.get("BENCH_STOP_AFTER") == "claims_per_block":  # anchor-growth / claims-per-block only
        B["meta"]["stopped_after"] = "claims_per_block"; save(); return
    # ---- 7. claims per tx: N claims (one dest wallet) in one tx
    B["claims"]["multi"] = {}
    per = sorted(B["blocks"]["pack"]["claim_gas"])[len(B["blocks"]["pack"]["claim_gas"]) // 2] if B["blocks"]["pack"]["claim_gas"] else 300_000
    left_notes = len(claimable)
    est_max = int((MAINNET_MAX_GAS - 150_000) // per)
    sizes = [10, 50]
    big = min(est_max - 2, left_notes - 60 - 4)
    if big > 50: sizes.append(big)
    for nclaims in sizes:
        name, w = fresh_wallet()
        ns = [claimable.pop() for _ in range(nclaims)]
        t0 = time.time()
        with cf.ThreadPoolExecutor(PROVERS) as ex: ps = list(ex.map(lambda x: prove(x, w, root), ns))
        gas = min(MAINNET_MAX_GAS, int(per * nclaims * 1.08) + 200_000)
        g = relayer.send([feegrant_msg(w, gas)], 200_000); assert g["code"] == 0
        a = Acct(name)
        r = a.send([exe(w, contract, claim_msg(p)) for p in ps], gas, granter=relayer.addr)
        B["claims"]["multi"][nclaims] = {"code": r["code"], "gas_used": r.get("gas_used"), "gas_wanted": gas, "tx_bytes": r.get("tx_bytes") or r.get("bytes"),
                                         "log": r.get("raw_log", "")[:200], "height": r.get("height"),
                                         "finalize_ms": None, "prove_wall_secs": round(time.time() - t0, 1)}
        save(); log("multi-claim", nclaims, B["claims"]["multi"][nclaims])
    time.sleep(3)
    for k, v in B["claims"]["multi"].items():
        if v.get("height"): v["finalize_ms"] = fb_ms(v["height"])
    # break point: one claim more than fits under max_gas is refused at CheckTx (gas wanted > block max_gas)
    name, w = fresh_wallet(); a_ = Acct(name) if False else None
    over = {"note": "gas_wanted %d > block max_gas" % (MAINNET_MAX_GAS + 1)}
    try:
        ns = [claimable.pop()]
        p = prove(ns[0], w, root)
        g = relayer.send([feegrant_msg(w, MAINNET_MAX_GAS + 1)], 200_000)
        a = Acct(name)
        r = a.send([exe(w, contract, claim_msg(p))], MAINNET_MAX_GAS + 1, granter=relayer.addr, wait=False)
        over.update({"code": r["code"], "log": r.get("raw_log", "")[:200]})
    except Exception as e: over["error"] = str(e)[:200]
    B["claims"]["over_block_gas"] = over; save()

    # ---- 8. path query gas: bisection over app.toml query_gas_limit (restarts the bench node)
    idn, i = claimable[-1]
    q = {"registration_path": {"key_com": h2b64(idn.kcs[i]), "anchor": b64(root)}}
    q2 = {"registration_tree": {}}
    qres = {}
    for name_, qq in (("registration_path", q), ("registration_tree", q2)):
        lo, hi = 0, 3_000_000
        def ok_at(lim):
            stop(); set_app("query_gas_limit", str(lim)); start()
            try: smart(contract, qq); return True
            except Exception: return False
        if not ok_at(hi): qres[name_] = {"over": hi}; continue
        while hi - lo > max(2000, hi // 50):
            mid = (lo + hi) // 2
            if ok_at(mid): hi = mid
            else: lo = mid
        qres[name_] = {"gas_le": hi, "gas_gt": lo}
        log("query gas", name_, qres[name_])
    stop(); set_app("query_gas_limit", "3000000"); start()
    B["query"]["gas_bisection_final"] = dict(qres, leaves=leaves[0], anchors=na, limit_default=3_000_000)
    save()
    stop_ev.set()
    B["meta"]["total_secs"] = round(time.time() - t_start, 1); save(); log("DONE")

if __name__ == "__main__":
    if sys.argv[1:] == ["setup"]: setup()
    elif sys.argv[1:] == ["run"]: main_run()
    elif sys.argv[1:] == ["start"]: start()
    elif sys.argv[1:] == ["stop"]: stop()
    else: print(__doc__); sys.exit(2)
