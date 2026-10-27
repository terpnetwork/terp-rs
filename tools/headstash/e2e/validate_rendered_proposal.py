import json, subprocess, time, urllib.request, os
T = "/tmp/terpd-v640-build/terpd"; H = "/tmp/headstash-govcircuit"; CID = "govcircuit-local"; N = "tcp://127.0.0.1:46767"
F = os.path.expanduser("~/abstract/terp-core/crates/terp-rs-headstash-membership-vk/tools/headstash/deploy/rendered-example/submit-proposal.json")
VAL = "terp1gv0zwaywewg69es7j7eh74tgmwdalg6nnmlh6y"
K = ["--keyring-backend", "test", "--home", H, "--chain-id", CID]
def sh(*a): r = subprocess.run([T, *a], capture_output=True, text=True); return r.returncode, r.stdout, r.stderr
rc, out, err = sh("tx", "gov", "submit-proposal", F, "--from", VAL, "--generate-only", "--offline", "--account-number", "0", "--sequence", "0", "--keyring-backend", "test", "--home", H)
if rc == 0:
    m = json.loads(out)["body"]["messages"][0]["messages"]
    print("generate-only --offline: OK,", len(m), "msgs:", [x["@type"].split(".")[-1] for x in m])
else: print("generate-only --offline FAIL", err[-400:])
def tx(*a):
    rc, out, err = sh("tx", *a, "--from", "val", *K, "--node", N, "--gas", "60000000", "--fees", "3000000uterp", "-y", "-o", "json")
    d = json.loads(out); assert d["code"] == 0, d
    for _ in range(20):
        time.sleep(1); rc, o, e = sh("q", "tx", d["txhash"], "--node", N, "-o", "json")
        if rc == 0: r = json.loads(o); assert r["code"] == 0, r["raw_log"]; return r
    raise SystemExit("tx not found")
tx("distribution", "fund-community-pool", "5000000uterp,5000000000uthiol")
print("community pool funded locally (5 TERP + 5000 THIOL)")
r = tx("gov", "submit-proposal", F)
pid = [a["value"] for e in r["events"] if e["type"] == "submit_proposal" for a in e["attributes"] if a["key"] == "proposal_id"][0]
print("submitted proposal", pid, "gas", r["gas_used"])
tx("gov", "vote", pid, "yes")
for _ in range(40):
    time.sleep(1); st = json.loads(sh("q", "gov", "proposal", pid, "--node", N, "-o", "json")[1]); st = st.get("proposal", st)
    if st["status"] != "PROPOSAL_STATUS_VOTING_PERIOD": break
print("status", st["status"], st.get("failed_reason", ""))
h = int(json.load(urllib.request.urlopen("http://127.0.0.1:46767/status"))["result"]["sync_info"]["latest_block_height"])
for x in range(h - 30, h + 1):
    for e in json.load(urllib.request.urlopen(f"http://127.0.0.1:46767/block_results?height={x}"))["result"].get("finalize_block_events") or []:
        if e["type"] in ("store_full_circuit", "active_proposal") or (e["type"] == "transfer" and any(a["value"].startswith("4800") or a["value"].startswith("2400") for a in e["attributes"])):
            print(x, e["type"], {a["key"]: a["value"][:50] for a in e["attributes"] if a["key"] != "mode"})
p = json.load(open(F))
for m in p["messages"]:
    if m["@type"].endswith("MsgCommunityPoolSpend") and m["amount"][0]["denom"] == "uthiol":
        b = json.loads(sh("q", "bank", "balances", m["recipient"], "--node", N, "-o", "json")[1])
        print("operator", m["recipient"], "balance", b["balances"], "expected", m["amount"][0]["amount"])
