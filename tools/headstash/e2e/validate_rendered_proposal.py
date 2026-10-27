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
camp_f = os.path.join(os.path.dirname(F), "02-set-epoch-campaign-open.json")
if os.path.exists(camp_f):
    c = json.load(open(camp_f))["set_epoch_campaign"]["campaign"]
    check("campaign burns unclaimed (no sweep recipient)", c.get("burn_unclaimed") is True and c.get("sweep_recipient") == "",
          {"burn_unclaimed": c.get("burn_unclaimed"), "sweep_recipient": c.get("sweep_recipient")})
    r_ = c["registration"]
    check("anchor_interval 16 blocks, min_anonymity_set 1024", r_["anchor_interval"] == 16 and r_["min_anonymity_set"] == 1024,
          {k: r_[k] for k in ("anchor_interval", "min_anonymity_set")})
    check("cutoff_height set", isinstance(c["cutoff_height"], int) and c["cutoff_height"] > 24_118_282, c["cutoff_height"])
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
pool = {}
for c in json.loads(sh("q", "distribution", "community-pool", "--node", N, "-o", "json")[1]).get("pool", []):
    d, a = (c["denom"], c["amount"]) if isinstance(c, dict) else (c.lstrip("0123456789."), c[:len(c) - len(c.lstrip("0123456789."))])
    pool[d] = pool.get(d, 0) + int(float(a))
short = {d: v - pool.get(d, 0) for d, v in need.items() if v > pool.get(d, 0)}
fund = ",".join(f"{v}{d}" for d, v in sorted(short.items()))
if fund: tx("distribution", "fund-community-pool", fund)
print("community pool topped up locally to cover the proposal spends (reruns reuse leftovers):", fund or "nothing needed", "need", need)
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
