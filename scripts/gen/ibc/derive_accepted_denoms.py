#!/usr/bin/env python3
"""Predetermine ICS-20 ibc/ denoms for multi-hop paths.
ibc denom = 'ibc/' + UPPER(hex(sha256("{port}/{chan}/.../{base}")))
Trace order: the RECEIVING chain's channel first (outermost hop last-applied)."""
import hashlib, json, sys, pathlib
try:
    import tomllib  # py>=3.11
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib

def ibc_denom(path: str, base: str) -> str:
    full = f"{path}/{base}" if path else base
    return "ibc/" + hashlib.sha256(full.encode()).hexdigest().upper()

def trace(hops):  # hops listed receiving-side first, e.g. ["channel-16","channel-4"]
    return "/".join(f"transfer/{c}" for c in hops)

# Self-tests: (path, base, expected) taken live from Terp /ibc/apps/transfer/v1/denoms/{hash}
SELF_TESTS = [  # keep in sync with tests/src/ibc/accepted.rs SELF_TEST_VECTORS
 ("transfer/channel-750", "uusdc", "ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
 ("transfer/channel-79703", "upenumbra", "ibc/0FA9232B262B89E77D1335D54FB1E1F506A92A7E4B51524B400DC69C68D28372"),
 ("transfer/channel-9", "uakt", "ibc/ED3433FEFECE0B9EC584F0EBAB31E8AB6A7A187537F3E5B1D8A80998986E7ACF"),
 ("transfer/channel-11/transfer/channel-73", "ubtsg", "ibc/5FFFDD494CE1478FB87727208AE1F94EE80467037291F02AC3BA108CA8868028"),
 ("transfer/channel-11", "factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC",
  "ibc/9FC418778218616ADEC741B8E402C856EBF2AADAD93A8D7EA322F798480F641C"),
]

def self_test():
    for p, b, exp in SELF_TESTS:
        got = ibc_denom(p, b)
        assert got == exp, (p, b, got, exp)
    return len(SELF_TESTS)

def main(routes_file, out_json):
    n = self_test()
    cfg = tomllib.loads(pathlib.Path(routes_file).read_text())
    assets = cfg["assets"]; out = {"chain_id": cfg["dest_chain"], "spec": "ICS-20 sha256(path/base)",
                                   "self_tests_passed": n, "assets": {}}
    for name, a in assets.items():
        rows = []
        for r in a["routes"]:
            p = trace(r["hops"])
            rows.append({"via": r["via"], "trace_path": p, "base_denom": a["base"],
                         "ibc_denom": ibc_denom(p, a["base"]), "status": r["status"], "note": r.get("note","")})
        out["assets"][name] = {"canonical": a["canonical"], "origin": a["origin"], "base_denom": a["base"], "variants": rows}
    out["shitstrap_accepted"] = {k: [v["ibc_denom"] for v in a["variants"] if v["status"] in ("live", "terp-live")]
                                 for k, a in out["assets"].items()}
    pathlib.Path(out_json).write_text(json.dumps(out, indent=2) + "\n")
    for name, a in out["assets"].items():
        for v in a["variants"]:
            print(f"{name:12} {v['via']:28} {v['status']:9} {v['trace_path']:62} {v['ibc_denom']}")

if __name__ == "__main__":
    main(*(sys.argv[1:] or ["public/ibc-accepted/routes.toml", "public/ibc-accepted/accepted-denoms.json"]))
