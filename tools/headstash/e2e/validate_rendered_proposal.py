"""Local-only (127.0.0.1, own test chain): submit deploy/rendered-example/submit-proposal.json on a v6.4.0
single node (gov_circuit_v640.py setup), vote, and check it passes with every message executed.
Direct path: also asserts there is no hashmerchant / lane message in the proposal."""
import json, subprocess, sys, time, urllib.request, os
T = "/tmp/terpd-v640-build/terpd"; H = "/tmp/headstash-govcircuit"; CID = "govcircuit-local"; N = "tcp://127.0.0.1:46767"
F = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser(
    "~/abstract/terp-core/crates/terp-rs-headstash-membership-vk/tools/headstash/deploy/rendered-example/submit-proposal.json")
K = ["--keyring-backend", "test", "--home", H, "--chain-id", CID]
def sh(*a): r = subprocess.run([T, *a], capture_output=True, text=True); return r.returncode, r.stdout, r.stderr
fails = []
def check(name, ok, info=""):
    print(("PASS " if ok else "FAIL ") + name, info)
    if not ok: fails.append(name)
p = json.load(open(F))
types = [m["@type"] for m in p["messages"]]
check("no lane/hashmerchant messages", not any("hashmerchant" in t for t in types), [t.split(".")[-1] for t in types])
need = {}
for m in p["messages"]:
    if m["@type"].endswith("MsgCommunityPoolSpend"):
        for c in m["amount"]: need[c["denom"]] = need.get(c["denom"], 0) + int(c["amount"])
def tx(*a):
    rc, out, err = sh("tx", *a, "--from", "val", *K, "--node", N, "--gas", "60000000", "--fees", "3000000uterp", "-y", "-o", "json")
    d = json.loads(out); assert d["code"] == 0, d
    for _ in range(20):
        time.sleep(1); rc, o, e = sh("q", "tx", d["txhash"], "--node", N, "-o", "json")
        if rc == 0: r = json.loads(o); assert r["code"] == 0, r["raw_log"]; return r
    raise SystemExit("tx not found")
def bal(addr):
    b = json.loads(sh("q", "bank", "balances", addr, "--node", N, "-o", "json")[1])["balances"]
    return {c["denom"]: int(c["amount"]) for c in b}
fund = ",".join(f"{v}{d}" for d, v in sorted(need.items()))
tx("distribution", "fund-community-pool", fund)
print("community pool funded locally with exactly the proposal spends:", fund)
before = {m["recipient"]: bal(m["recipient"]) for m in p["messages"] if m["@type"].endswith("MsgCommunityPoolSpend")}
r = tx("gov", "submit-proposal", F)
pid = [a["value"] for e in r["events"] if e["type"] == "submit_proposal" for a in e["attributes"] if a["key"] == "proposal_id"][0]
print("submitted proposal", pid, "gas", r["gas_used"])
tx("gov", "vote", pid, "yes")
for _ in range(40):
    time.sleep(1); st = json.loads(sh("q", "gov", "proposal", pid, "--node", N, "-o", "json")[1]); st = st.get("proposal", st)
    if st["status"] != "PROPOSAL_STATUS_VOTING_PERIOD": break
check("proposal passed", st["status"] == "PROPOSAL_STATUS_PASSED", st["status"] + " " + st.get("failed_reason", ""))
h = int(json.load(urllib.request.urlopen("http://127.0.0.1:46767/status"))["result"]["sync_info"]["latest_block_height"])
evs = [e for x in range(max(1, h - 40), h + 1)
       for e in json.load(urllib.request.urlopen(f"http://127.0.0.1:46767/block_results?height={x}"))["result"].get("finalize_block_events") or []]
sfc = [e for e in evs if e["type"] == "store_full_circuit"]
check("store_full_circuit emitted", bool(sfc), [{a["key"]: a["value"][:64] for a in e["attributes"] if a["key"] != "mode"} for e in sfc])
for m in p["messages"]:
    if m["@type"].endswith("MsgCommunityPoolSpend"):
        got = bal(m["recipient"]); c = m["amount"][0]
        delta = got.get(c["denom"], 0) - before[m["recipient"]].get(c["denom"], 0)
        check("pool spend " + m["recipient"][:14] + "...", delta == int(c["amount"]), f"+{delta}{c['denom']} expected {c['amount']}")
print("RESULT", "OK" if not fails else "FAILED: " + ", ".join(fails))
sys.exit(1 if fails else 0)
