#!/usr/local/bin/python3
"""Local-only: v6.4.0 terpd single node, circuit_upload_access Nobody, gov proposal MsgStoreFullCircuit (vk_id 3)."""
import base64, json, os, subprocess, sys, time, hashlib, urllib.request
from pathlib import Path
T = "/tmp/terpd-v640-build/terpd"
H = Path("/tmp/headstash-govcircuit")
CID = "govcircuit-local"
CIRC = Path("/tmp/headstash-mvk-chain/run-1793063360/circuit")
P = {"rpc": 46767, "p2p": 46766, "proxy": 46768, "pprof": 46161, "prom": 46770, "rest": 41427, "grpc": 49191}
RPC = f"tcp://127.0.0.1:{P['rpc']}"
GOV = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq"
def run(*a, check=True, inp=None):
    r = subprocess.run([T, *a, "--home", str(H)], capture_output=True, text=True, input=inp)
    if check and r.returncode: raise SystemExit(f"FAIL {a[:3]}: {r.stderr[-800:]}")
    return r.stdout
def q(*a): return json.loads(run("q", *a, "--node", RPC, "-o", "json"))
def tx(*a):
    out = json.loads(run("tx", *a, "--from", "val", "--keyring-backend", "test", "--chain-id", CID, "--node", RPC,
        "--gas", "auto", "--gas-adjustment", "1.5", "--fees", "200000uterp", "-y", "-o", "json"))
    assert out["code"] == 0, out
    for _ in range(30):
        time.sleep(1)
        try: r = q("tx", out["txhash"]); break
        except SystemExit: pass
    assert r["code"] == 0, r.get("raw_log")
    return r
if sys.argv[1:] == ["setup"]:
    if H.exists(): subprocess.run(["rm", "-rf", str(H)])
    os.umask(0o077)
    run("init", "govc", "--chain-id", CID, "--default-denom", "uterp")
    run("keys", "add", "val", "--keyring-backend", "test", "--output", "json", inp="")
    addr = run("keys", "show", "val", "-a", "--keyring-backend", "test").strip()
    run("genesis", "add-genesis-account", addr, "100000000000uterp,100000000000uthiol", "--keyring-backend", "test")
    run("genesis", "gentx", "val", "1000000000uterp", "--chain-id", CID, "--keyring-backend", "test")
    run("genesis", "collect-gentxs")
    g = json.loads((H / "config/genesis.json").read_text())
    wp = g["app_state"]["wasm"]["params"]
    print("wasm params keys:", list(wp))
    wp["circuit_upload_access"] = {"permission": "Nobody", "addresses": []}
    gp = g["app_state"]["gov"]["params"]
    gp["voting_period"] = "15s"; gp["expedited_voting_period"] = "10s"; gp["max_deposit_period"] = "30s"
    gp["min_deposit"] = [{"denom": "uterp", "amount": "1000000"}]
    gp["expedited_min_deposit"] = [{"denom": "uterp", "amount": "2000000"}]
    (H / "config/genesis.json").write_text(json.dumps(g, indent=1))
    c = (H / "config/config.toml").read_text()
    for o, n in [('tcp://127.0.0.1:26658', f'tcp://127.0.0.1:{P["proxy"]}'), ('tcp://127.0.0.1:26657', f'tcp://127.0.0.1:{P["rpc"]}'),
                 ('tcp://0.0.0.0:26656', f'tcp://127.0.0.1:{P["p2p"]}'), ('localhost:6060', f'127.0.0.1:{P["pprof"]}'),
                 ('":26660"', f'"127.0.0.1:{P["prom"]}"'), ('timeout_commit = "5s"', 'timeout_commit = "1s"')]:
        c = c.replace(o, n)
    (H / "config/config.toml").write_text(c)
    a = (H / "config/app.toml").read_text()
    for o, n in [('minimum-gas-prices = "0stake"', 'minimum-gas-prices = "0uterp"'), ('minimum-gas-prices = ""', 'minimum-gas-prices = "0uterp"'),
                 ('tcp://localhost:1317', f'tcp://127.0.0.1:{P["rest"]}'), ('localhost:9090', f'127.0.0.1:{P["grpc"]}')]:
        a = a.replace(o, n)
    (H / "config/app.toml").write_text(a)
    for f in ("config.toml", "app.toml"):
        for l in (H / "config" / f).read_text().splitlines():
            s = l.strip()
            if s and not s.startswith("#") and ("0.0.0.0" in s or "[::]" in s): raise SystemExit(f"public bind {f}: {s}")
    run("genesis", "validate")
    print("setup ok val", addr); sys.exit()
# test phase
pb, vb = (CIRC / "params.bin").read_bytes(), (CIRC / "vk_body.bin").read_bytes()
assert hashlib.sha256(pb + vb).hexdigest().startswith("34d11437"), "blob hash"
print("params.circuit_upload_access:", (lambda d: d.get("params", d).get("circuit_upload_access"))(q("wasm", "params")))
addr = run("keys", "show", "val", "-a", "--keyring-backend", "test").strip()
# 1) direct non-gov upload must fail under Nobody (simulate only, no deposit)
r = subprocess.run([T, "tx", "wasm", "store-full-circuit", str(CIRC/"params.bin"), str(CIRC/"vk_body.bin"), "--k", "11", "--circuit-type", "0", "--curve-type", "0",
    "--from", "val", "--keyring-backend", "test", "--chain-id", CID, "--node", RPC, "--home", str(H), "--gas", "auto", "--dry-run"], capture_output=True, text=True)
print("direct user store under Nobody: rc", r.returncode, (r.stderr or r.stdout).strip().splitlines()[-1][:300])
msg = {"@type": "/cosmwasm.wasm.v1.MsgStoreFullCircuit", "sender": GOV,
       "auth": {"circuit_type": 0, "curve_type": 0, "k": 11},
       "params_body": base64.b64encode(pb).decode(), "vk_body": base64.b64encode(vb).decode()}
prop = {"messages": [msg], "metadata": "", "deposit": "1000000uterp", "title": "store vk_id3", "summary": "local test", "expedited": False}
Path("/tmp/hsdirect/govc-prop.json").write_text(json.dumps(prop))
r = tx("gov", "submit-proposal", "/tmp/hsdirect/govc-prop.json")
pid = [a["value"] for e in r["events"] if e["type"] == "submit_proposal" for a in e["attributes"] if a["key"] == "proposal_id"][0]
print("proposal", pid, "submitted, gas", r["gas_used"])
tx("gov", "vote", pid, "yes")
for _ in range(40):
    time.sleep(1)
    st = q("gov", "proposal", pid)
    st = st.get("proposal", st)
    if st["status"] not in ("PROPOSAL_STATUS_VOTING_PERIOD", 2): break
print("status:", st["status"], "failed_reason:", st.get("failed_reason", ""))
if st["status"] in ("PROPOSAL_STATUS_PASSED", 3):
    h = run("q", "txs", "--query", "store_full_circuit.cs_checksum EXISTS", "--node", RPC, "-o", "json", check=False)
    import re
    out = subprocess.run(["grep", "-a", "-i", "store_full_circuit\\|circuit", str(H / "terpd.log")], capture_output=True, text=True).stdout.splitlines()[-5:]
    print("log:", *[l[:250] for l in out], sep="\n ")
