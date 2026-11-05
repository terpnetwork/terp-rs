#!/usr/bin/env python3
"""Probes on the filled bench chain after the run hit the ANCHORS item limit (local chain only, test keyring).
claim with a real anchor + junk proof (gas of the anchor scan before verify), accept_anchor retry, query gas bisection."""
import base64, json, os, secrets, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench_limits as bl
B = json.load(open(bl.RUN / "bench.json")); C = B["meta"]["contract"]
def save():
    json.dump(B, open(bl.RUN / "bench.json", "w"), indent=1)
out = {}
rt = bl.smart(C, {"registration_tree": {}})[0]
raw = bl.requests.get(bl.REST + "/cosmwasm/wasm/v1/contract/%s/smart/%s" % (C, base64.b64encode(b'{"registration_tree":{}}').decode()), timeout=30)
out["tree"] = {"count": rt["count"], "anchors": len(rt["anchors"]), "registration_tree_resp_bytes": len(raw.content),
               "anchors_vec_json_bytes_est": len(json.dumps([{"root": list(base64.b64decode(a["root"])), "leaf_count": a["leaf_count"]} for a in rt["anchors"]], separators=(",", ":")))}
s = bl.Acct("stranger"); a = bl.Acct("anchorer")
for label, root in (("latest", rt["anchors"][-1]["root"]), ("oldest", rt["anchors"][0]["root"]), ("unknown", base64.b64encode(secrets.token_bytes(32)).decode())):
    r = s.send([bl.exe(s.addr, C, {"claim_note": {"root": root, "nullifier": base64.b64encode(secrets.token_bytes(32)).decode(), "amount": 5000000000, "proof": base64.b64encode(b"\0" * 2752).decode()}})], 20_000_000)
    out["junk_claim_" + label] = {k: r.get(k) for k in ("code", "gas_used", "raw_log")}; out["junk_claim_" + label]["raw_log"] = (r.get("raw_log") or "")[:240]
    print(label, out["junk_claim_" + label], flush=True)
old = json.load(open(bl.RUN / "bench.json")).get("full_probe", {})
out = dict(old, **{k: v for k, v in out.items() if k.startswith("junk")})
B["full_probe"] = out; save(); print("PROBE DONE", flush=True)
