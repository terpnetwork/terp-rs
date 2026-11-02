#!/usr/bin/env python3
"""Point a fresh terpd home at loopback and register the headstash claim chain.

Reads and writes genesis plus the two config files. Does not print keys.
The second argument is the base64 ed25519 OracleSource pubkey. It is public.
"""

import base64
import json
import sys
from pathlib import Path

HOME = Path(sys.argv[1])
PUBKEY_B64 = sys.argv[2]
GENESIS = HOME / "config" / "genesis.json"
CONFIG = HOME / "config" / "config.toml"
APP = HOME / "config" / "app.toml"

REPLACEMENTS = {
    CONFIG: [
        ('proxy_app = "tcp://127.0.0.1:26658"', 'proxy_app = "tcp://127.0.0.1:36668"'),
        ('laddr = "tcp://127.0.0.1:26657"', 'laddr = "tcp://127.0.0.1:36667"'),
        ('laddr = "tcp://0.0.0.0:26656"', 'laddr = "tcp://127.0.0.1:36666"'),
        ('pprof_laddr = "localhost:6060"', 'pprof_laddr = "127.0.0.1:36061"'),
        ('timeout_commit = "5s"', 'timeout_commit = "1s"'),
        ('prometheus_listen_addr = ":26660"', 'prometheus_listen_addr = "127.0.0.1:26670"'),
    ],
    APP: [
        ('minimum-gas-prices = "0stake"', 'minimum-gas-prices = "0.025uterp"'),
        ('address = "tcp://localhost:1317"', 'address = "tcp://127.0.0.1:31327"'),
        ('address = "localhost:9090"', 'address = "127.0.0.1:39091"'),
    ],
}


def patch_text(path: Path, pairs: list[tuple[str, str]]) -> None:
    text = path.read_text()
    for old, new in pairs:
        if old not in text:
            raise SystemExit(f"missing config line in {path.name}: {old}")
        text = text.replace(old, new, 1)
    if "[hashmerchant]" not in text and path == APP:
        text += (
            "\n[hashmerchant]\n"
            'sidecar-url = "http://127.0.0.1:19080"\n'
            'sidecar-timeout = "2s"\n'
        )
    path.write_text(text)


def refuse_public_bind(path: Path) -> None:
    for lineno, line in enumerate(path.read_text().splitlines(), 1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if "0.0.0.0" in stripped or "[::]" in stripped:
            raise SystemExit(f"public bind in {path.name}:{lineno}: {stripped}")


def patch_genesis() -> None:
    raw = base64.b64decode(PUBKEY_B64, validate=True)
    if len(raw) != 32:
        raise SystemExit(f"custody pubkey is {len(raw)} bytes")
    genesis = json.loads(GENESIS.read_text())
    abci = genesis["consensus"]["params"]["abci"]
    abci["vote_extensions_enable_height"] = "2"
    merchant = genesis["app_state"]["hashmerchant"]
    merchant["registered_chains"] = [
        {
            "chain_uid": "headstash-claim",
            "name": "headstash-claim",
            "rpc_endpoints": [],
            "hash_algos": ["headstash-pending", "headstash-accepted"],
            "enabled": True,
            "oracle_sources": [
                {
                    "source_id": "headstash-batch",
                    "kind": "ORACLE_SOURCE_KIND_HTTP",
                    "endpoint": "http://127.0.0.1:19080",
                    "merkle_prefix": [],
                    "authenticator": {
                        "pubkey": PUBKEY_B64,
                        "algorithm": "ed25519",
                        "scope": "headstash-batch",
                    },
                    "enabled": True,
                }
            ],
        }
    ]
    params = merchant["params"]
    if params.get("escrow_denom") != "uterp":
        raise SystemExit(f"escrow denom is {params.get('escrow_denom')}")
    if params.get("quorum_fraction") != "0.667000000000000000":
        raise SystemExit(f"quorum is {params.get('quorum_fraction')}")
    bond = genesis["app_state"]["staking"]["params"]["bond_denom"]
    if bond != "uterp":
        raise SystemExit(f"bond denom is {bond}")
    GENESIS.write_text(json.dumps(genesis, indent=2) + "\n")
    loaded = json.loads(GENESIS.read_text())
    source = loaded["app_state"]["hashmerchant"]["registered_chains"][0]["oracle_sources"][0]
    if source["source_id"] != "headstash-batch" or source["enabled"] is not True:
        raise SystemExit("oracle source was not stored")
    auth = source["authenticator"]
    if auth["algorithm"] != "ed25519" or auth["scope"] != "headstash-batch":
        raise SystemExit("oracle authenticator was not stored")
    if base64.b64decode(auth["pubkey"], validate=True) != raw:
        raise SystemExit("oracle pubkey was not stored")


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: patch_epoch_genesis.py HOME PUBKEY_B64")
    for path, pairs in REPLACEMENTS.items():
        patch_text(path, pairs)
        refuse_public_bind(path)
    patch_genesis()
    print(
        "genesis patched vote_extensions_enable_height=2 "
        "chain=headstash-claim source=headstash-batch"
    )


if __name__ == "__main__":
    main()
