#!/usr/bin/env python3
"""Loopback ports + headstash-claim oracle registration for the membership-vk e2e chain.

usage: patch_genesis.py HOME CUSTODY_PUB_HEX LANE_URL
Edits genesis/config/app. Prints nothing secret (the custody pubkey is public).
"""
import base64, json, sys
from pathlib import Path

HOME, PUB_HEX, LANE = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
assert LANE.startswith("http://127.0.0.1:"), "lane must be loopback"
CONFIG, APP, GENESIS = HOME / "config/config.toml", HOME / "config/app.toml", HOME / "config/genesis.json"
P = {"rpc": 46667, "p2p": 46666, "proxy": 46668, "pprof": 46061, "prom": 46670, "rest": 41327, "grpc": 49091}

REPL = {
    CONFIG: [
        ('proxy_app = "tcp://127.0.0.1:26658"', f'proxy_app = "tcp://127.0.0.1:{P["proxy"]}"'),
        ('laddr = "tcp://127.0.0.1:26657"', f'laddr = "tcp://127.0.0.1:{P["rpc"]}"'),
        ('laddr = "tcp://0.0.0.0:26656"', f'laddr = "tcp://127.0.0.1:{P["p2p"]}"'),
        ('pprof_laddr = "localhost:6060"', f'pprof_laddr = "127.0.0.1:{P["pprof"]}"'),
        ('timeout_commit = "5s"', 'timeout_commit = "1s"'),
        ('prometheus_listen_addr = ":26660"', f'prometheus_listen_addr = "127.0.0.1:{P["prom"]}"'),
    ],
    APP: [
        ('minimum-gas-prices = "0stake"', 'minimum-gas-prices = "0.025uterp"'),
        ('minimum-gas-prices = ""', 'minimum-gas-prices = "0.025uterp"'),
        ('address = "tcp://localhost:1317"', f'address = "tcp://127.0.0.1:{P["rest"]}"'),
        ('address = "localhost:9090"', f'address = "127.0.0.1:{P["grpc"]}"'),
    ],
}
for path, pairs in REPL.items():
    t = path.read_text()
    for old, new in pairs:
        if old in t:
            t = t.replace(old, new, 1)
        elif new not in t and "minimum-gas-prices" not in old:
            raise SystemExit(f"missing line in {path.name}: {old}")
    if path == APP:
        i = t.index("[api]")
        j = t.index("enable = ", i)
        t = t[:j] + "enable = true" + t[j + len("enable = false"):] if t.startswith("enable = false", j) else t
        if "[hashmerchant]" not in t:
            t += f'\n[hashmerchant]\nsidecar-url = "{LANE}"\nsidecar-timeout = "2s"\n'
    for n, line in enumerate(t.splitlines(), 1):
        s = line.strip()
        if s and not s.startswith("#") and ("0.0.0.0" in s or "[::]" in s):
            raise SystemExit(f"public bind {path.name}:{n}: {s}")
    path.write_text(t)
if '0.025uterp' not in APP.read_text():
    raise SystemExit("min gas price not set")

raw = bytes.fromhex(PUB_HEX)
assert len(raw) == 32
pub_b64 = base64.b64encode(raw).decode()
g = json.loads(GENESIS.read_text())
g["consensus"]["params"]["abci"]["vote_extensions_enable_height"] = "2"
import os
if os.environ.get("GOV_ROLES") == "1" or os.environ.get("DAO_ROLES") == "1":  # mainnet-like roles: only gov stores circuits
    g["app_state"]["wasm"]["params"]["circuit_upload_access"] = {"permission": "Nobody", "addresses": []}
    gp = g["app_state"]["gov"]["params"]
    gp.update(voting_period="15s", expedited_voting_period="10s", max_deposit_period="300s",
              min_deposit=[{"denom": "uterp", "amount": "10000000"}], expedited_min_deposit=[{"denom": "uterp", "amount": "20000000"}],
              min_initial_deposit_ratio="0.500000000000000000")
if os.environ.get("DAO_ROLES") == "1":  # morocco-1: no tokenfactory denom creation fee (gas consume only)
    g["app_state"]["tokenfactory"]["params"]["denom_creation_fee"] = []
m = g["app_state"]["hashmerchant"]
m["registered_chains"] = [{
    "chain_uid": "headstash-claim", "name": "headstash-claim", "rpc_endpoints": [],
    "hash_algos": ["headstash-pending", "headstash-accepted"], "enabled": True,
    "oracle_sources": [{
        "source_id": "headstash-batch", "kind": "ORACLE_SOURCE_KIND_HTTP", "endpoint": LANE,
        "merkle_prefix": [], "enabled": True,
        "authenticator": {"pubkey": pub_b64, "algorithm": "ed25519", "scope": "headstash-batch"},
    }],
}]
assert m["params"].get("escrow_denom") == "uterp", m["params"].get("escrow_denom")
assert g["app_state"]["staking"]["params"]["bond_denom"] == "uterp"
GENESIS.write_text(json.dumps(g, indent=2) + "\n")
print("genesis patched: VE height 2, headstash-claim/headstash-batch ed25519, lane", LANE)
