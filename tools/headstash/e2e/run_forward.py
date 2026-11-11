#!/usr/bin/env python3
"""Headstash v2 ClaimNoteForward local e2e (loopback only, two chains + hermes).

Chain A: the usual local e2e chain (/tmp/headstash-mvk-chain, headstash-mvk-e2e).
Chain B: a SECOND terpd (/tmp/headstash-mvk-chainB, hs-fwd-b). It stands in for Penumbra:
it is a plain ICS-20 receiver, so this proves the Terp side (proof binding, allowlist,
callbacks, refunds), not Penumbra's shielded pool or Zafu address handling.

Runs run_direct.main() (real circuit, real halo2 proofs via prove_note_witness, the
optimized zk-api wasm) up to the positive direct claims, then FWD_HOOK below, then stops.
Throwaway test keyrings only (keyring-backend test under /tmp); no secret is printed.
"""
import json, os, subprocess, sys, time, re, base64
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
os.environ.setdefault("TOKEN", "existing")
import run_direct as rd
from run import R, step, terpd, q, tx, smart, attr, gas_used, rec, TERPD, HOME, CHAIN_ID

HB = "/tmp/headstash-mvk-chainB"; CB = "hs-fwd-b"
B_RPC = "http://127.0.0.1:46677"; B_NODE = "tcp://127.0.0.1:46677"
HERMES = os.environ.get("HERMES", os.path.expanduser("~/go/bin/hermes"))
HH = "/tmp/headstash-mvk-chainB/hermes"; HCFG = HH + "/config.toml"
hermes_proc = None

def run(*a, check=True, **kw):
    p = subprocess.run(list(a), capture_output=True, text=True, **kw)
    if check and p.returncode != 0: raise RuntimeError("%s: %s" % (a[:3], (p.stdout + p.stderr)[-400:]))
    return p.stdout

def bterpd(*a, check=True): return run(TERPD, *a, check=check)

def b_height():
    try:
        import urllib.request
        return int(json.load(urllib.request.urlopen(B_RPC + "/status", timeout=2))["result"]["sync_info"]["latest_block_height"])
    except Exception: return 0

def key_to_hermes(home, name, chain_id):
    """Fresh throwaway key -> hermes keyring. The mnemonic goes from terpd's JSON straight to a 0600 file; never printed."""
    run(TERPD, "keys", "delete", name, "-y", "--keyring-backend", "test", "--home", home, check=False)
    out = json.loads(run(TERPD, "keys", "add", name, "--keyring-backend", "test", "--home", home, "--output", "json"))
    os.makedirs(HH, exist_ok=True)
    f = os.path.join(HH, ".%s-%s.mn" % (chain_id, name))
    fd = os.open(f, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.write(fd, out["mnemonic"].encode()); os.close(fd); del out["mnemonic"]
    run(HERMES, "--config", HCFG, "keys", "add", "--chain", chain_id, "--key-name", "hermes", "--mnemonic-file", f, "--overwrite")
    return out["address"]

def chain_b():
    if b_height() >= 2: return
    os.makedirs(HB, exist_ok=True); os.chmod(HB, 0o700)
    if not os.path.exists(HB + "/config/genesis.json"):
        bterpd("init", "fwd-b", "--chain-id", CB, "--default-denom", "uterp", "--home", HB)
        for n in ("val", "recv"):
            bterpd("keys", "add", n, "--keyring-backend", "test", "--home", HB, "--output", "json")
        for n, amt in (("val", "100000000000"), ("recv", "1000000")):
            a = bterpd("keys", "show", n, "-a", "--keyring-backend", "test", "--home", HB).strip()
            bterpd("genesis", "add-genesis-account", a, amt + "uterp", "--home", HB, "--keyring-backend", "test")
        bterpd("genesis", "gentx", "val", "1000000uterp", "--chain-id", CB, "--home", HB, "--keyring-backend", "test")
        bterpd("genesis", "collect-gentxs", "--home", HB)
        run("python3", os.path.join(HERE, "patch_genesis.py"), HB, "58" + "66" * 31, "http://127.0.0.1:19181")
        for fn, subs in (("config.toml", [(r":4666(\d)\b", r":4667\1"), (r":2665(\d)\b", r":4667\1"), (r":46061\b", ":46071"), (r":6060\b", ":46071"), ('timeout_commit = "[^"]*"', 'timeout_commit = "1s"')]),
                         ("app.toml", [(r":41327\b", ":41337"), (r":1317\b", ":41337"), (r":49091\b", ":49101"), (r":49092\b", ":49102"), (r":9090\b", ":49101"), (r":9091\b", ":49102"), ("0.0.0.0", "127.0.0.1")])):
            p = os.path.join(HB, "config", fn); s = open(p).read()
            for a_, b_ in subs: s = re.sub(a_, b_, s)
            open(p, "w").write(s)
        bterpd("genesis", "validate", "--home", HB)
    log = open(HB + "/terpd.log", "a")
    pr = subprocess.Popen([TERPD, "start", "--home", HB], stdout=log, stderr=subprocess.STDOUT)
    open(HB + "/terpd.pid", "w").write(str(pr.pid))
    for _ in range(90):
        if b_height() >= 3: break
        time.sleep(1)
    step("fwd_chain_b_up", b_height() >= 3, "second terpd (stand-in for Penumbra) chain %s height %d" % (CB, b_height()))

def unbonding_secs(home):
    g = json.load(open(home + "/config/genesis.json"))
    return int(g["app_state"]["staking"]["params"]["unbonding_time"].rstrip("s"))

def hermes_cfg():
    os.makedirs(HH, exist_ok=True)
    def chain(cid, rpc, grpc, home):
        tp = max(120, unbonding_secs(home) * 2 // 3)
        return """
[[chains]]
id = '%s'
type = 'CosmosSdk'
rpc_addr = '%s'
grpc_addr = '%s'
event_source = { mode = 'pull', interval = '1s' }
rpc_timeout = '10s'
account_prefix = 'terp'
key_name = 'hermes'
store_prefix = 'ibc'
default_gas = 400000
max_gas = 6000000
gas_price = { price = 0.025, denom = 'uterp' }
gas_multiplier = 1.6
clock_drift = '10s'
max_block_time = '10s'
trusting_period = '%ds'
trust_threshold = '1/3'
address_type = { derivation = 'cosmos' }
""" % (cid, rpc, grpc, tp)
    s = """[global]
log_level = 'info'
[mode.clients]
enabled = true
refresh = true
misbehaviour = false
[mode.connections]
enabled = false
[mode.channels]
enabled = false
[mode.packets]
enabled = true
clear_interval = 5
clear_on_start = true
tx_confirmation = true
[rest]
enabled = false
host = '127.0.0.1'
port = 3710
[telemetry]
enabled = false
host = '127.0.0.1'
port = 3711
""" + chain(CHAIN_ID, "http://127.0.0.1:46667", "http://127.0.0.1:49091", HOME) + chain(CB, B_RPC, "http://127.0.0.1:49101", HB)
    open(HCFG, "w").write(s)

def hermes_start():
    global hermes_proc
    hermes_stop()
    log = open(HH + "/hermes.log", "a")
    hermes_proc = subprocess.Popen([HERMES, "--config", HCFG, "start"], stdout=log, stderr=subprocess.STDOUT)
    open(HH + "/hermes.pid", "w").write(str(hermes_proc.pid))
    time.sleep(8)

def hermes_stop():
    global hermes_proc
    if hermes_proc and hermes_proc.poll() is None:
        hermes_proc.terminate()
        try: hermes_proc.wait(15)
        except Exception: hermes_proc.kill()
    hermes_proc = None

def setup_ibc(OWNER):
    hermes_cfg()
    ha = key_to_hermes(HB + "/hermes-keys-a", "hermes", CHAIN_ID)
    hb = key_to_hermes(HB + "/hermes-keys-b", "hermes", CB)
    tx("owner", 200000, "bank", "send", OWNER, ha, "2000000000uterp")
    bterpd("tx", "bank", "send", "val", hb, "2000000000uterp", "--home", HB, "--keyring-backend", "test", "--chain-id", CB,
           "--node", B_NODE, "--gas-prices", "0.025uterp", "-y", "-o", "json")
    time.sleep(4)
    out = run(HERMES, "--config", HCFG, "--json", "create", "channel", "--a-chain", CHAIN_ID, "--b-chain", CB,
              "--a-port", "transfer", "--b-port", "transfer", "--new-client-connection", "--yes", check=False)
    open(HH + "/create-channel.json", "w").write(out)
    ch_a = ch_b = None
    for line in out.splitlines():
        try: j = json.loads(line)
        except Exception: continue
        res = j.get("result")
        if isinstance(res, dict) and "a_side" in res:
            ch_a = res["a_side"].get("channel_id"); ch_b = res["b_side"].get("channel_id")
    step("fwd_ibc_channel_open", bool(ch_a and ch_b), "A %s <-> B %s (hermes %s)" % (ch_a, ch_b, out[-200:] if not ch_a else "ok"))
    return ch_a, ch_b

def b_balances(addr):
    j = json.loads(bterpd("q", "bank", "balances", addr, "--node", B_NODE, "-o", "json"))
    return {c["denom"]: int(c["amount"]) for c in j.get("balances", [])}

def wait_for(pred, secs, every=2):
    t0 = time.time()
    while time.time() - t0 < secs:
        v = pred()
        if v: return v
        time.sleep(every)
    return pred()

def hook(L):
    contract, prove, A2, notes, OWNER = L["contract"], L["prove"], L["A2"], L["notes"], L["OWNER"]
    R["forward"] = {"note": "chain B is a second terpd standing in for Penumbra (ICS-20 receiver only)"}
    chain_b()
    ch_a, ch_b = setup_ibc(OWNER)
    if not ch_a: return
    R["forward"].update({"channel_a": ch_a, "channel_b": ch_b, "chain_b": CB})
    hermes_start()
    recv = bterpd("keys", "show", "recv", "-a", "--keyring-backend", "test", "--home", HB).strip()
    used = {(c["note"].split("_")[0], int(c["note"].split("_")[1])) for c in R["claims"]}
    free = [i for i in range(2, len(notes["evm"]) - 2) if ("evm", i) not in used]
    def pending(): return smart(contract, {"pending_forwards": {}})
    def tgt(ch, receiver, fb, memo=None): return {"channel_id": ch, "receiver": receiver, "memo": memo, "fallback": fb}
    def fprove(i, t):
        dest = smart(contract, {"forward_destination": {"forward": t}})
        pr, err = prove("evm", i, dest, A2)
        if not pr: raise RuntimeError("prove: " + err)
        assert pr["dest"] == dest
        return pr, dest
    def fmsg(pr, t): return {"claim_note_forward": {"root": pr["root"], "nullifier": pr["nullifier"], "amount": pr["amount"], "proof": pr["proof"], "forward": t}}
    G = 900000

    # allowlist: owner only
    rd.neg("fwd_set_channel_by_stranger", tx("stranger", 400000, "wasm", "execute", contract,
           json.dumps({"set_forward_channel": {"channel_id": ch_a, "timeout_seconds": 600, "enabled": True}})))
    res = rec("fwd_set_channel", tx("owner", 400000, "wasm", "execute", contract,
              json.dumps({"set_forward_channel": {"channel_id": ch_a, "timeout_seconds": 600, "enabled": True}})))
    step("fwd_owner_allows_channel", res.get("code") == 0, "%s timeout 600s %s" % (ch_a, res.get("raw_log", "")[:120]))

    # the e2e escrow only covers the direct-claim picks; top it up for the 4 forward claims below
    res = tx("owner", 200000, "bank", "send", OWNER, contract, "20000000000uterp")
    step("fwd_escrow_topup", res.get("code") == 0, "contract %d uterp" % rd.tbal(contract))

    # 1. happy path: proof bound to (chain, contract, channel, receiver, memo, fallback) -> ICS-20 -> ack ok
    i1 = free.pop(0); fb1 = rd.new_wallet("fwd_fb1")
    t1 = tgt(ch_a, recv, fb1)
    pr1, d1 = fprove(i1, t1)
    R["forward"]["dest_example"] = d1
    # 1a. tampered receiver / fallback / channel under the same real proof: rejected, nothing spent
    t_bad = dict(t1, receiver=bterpd("keys", "show", "val", "-a", "--keyring-backend", "test", "--home", HB).strip())
    rd.neg("fwd_tampered_receiver_real_proof", tx("stranger", G, "wasm", "execute", contract, json.dumps(fmsg(pr1, t_bad))), "invalid note proof")
    rd.neg("fwd_tampered_fallback_real_proof", tx("stranger", G, "wasm", "execute", contract, json.dumps(fmsg(pr1, dict(t1, fallback=OWNER)))), "invalid note proof")
    rd.neg("fwd_tampered_memo_real_proof", tx("stranger", G, "wasm", "execute", contract, json.dumps(fmsg(pr1, dict(t1, memo='{"x":1}')))), "invalid note proof")
    rd.neg("fwd_unallowed_channel", tx("stranger", G, "wasm", "execute", contract, json.dumps(fmsg(pr1, dict(t1, channel_id="channel-99")))), "channel not allowed")
    c0 = rd.tbal(contract); b0 = b_balances(recv)
    res = rec("fwd_claim_ok", tx("stranger", G, "wasm", "execute", contract, json.dumps(fmsg(pr1, t1))))
    seq1 = None
    for ev in res.get("events", []):
        if ev.get("type") == "send_packet":
            for a in ev.get("attributes", []):
                if a["key"] == "packet_sequence": seq1 = int(a["value"])
    step("fwd_claim_tx_ok", res.get("code") == 0 and seq1 is not None and c0 - rd.tbal(contract) == pr1["amount"],
         "note evm_%d amount %d seq %s gas %d %s" % (i1, pr1["amount"], seq1, gas_used(res), res.get("raw_log", "")[:200]))
    R["forward"]["gas_claim_forward"] = gas_used(res)
    got = wait_for(lambda: next((v for d, v in b_balances(recv).items() if d.startswith("ibc/") and v - b0.get(d, 0) == pr1["amount"]), None), 90)
    step("fwd_received_on_chain_b", bool(got), "recv %s ibc balance %s" % (recv, b_balances(recv)))
    cleared = wait_for(lambda: len(pending()) == 0, 60)
    step("fwd_ack_success_clears_pending_no_refund", cleared and rd.tbal(fb1) == 0, "pending %s fallback bal %d" % (pending(), rd.tbal(fb1)))
    rd.neg("fwd_nullifier_replay", tx("stranger", G, "wasm", "execute", contract, json.dumps(fmsg(pr1, t1))), "nullifier")
    R["claims"].append({"note": "evm_%d" % i1, "amount": pr1["amount"], "forward": True})

    # 2. ack error: receiver is not a valid chain-B address -> B writes an error ack -> refund to the bound fallback
    i2 = free.pop(0); fb2 = rd.new_wallet("fwd_fb2")
    t2 = tgt(ch_a, "notavalidaddress", fb2)
    pr2, _ = fprove(i2, t2)
    res = rec("fwd_claim_ackerr", tx("stranger", G, "wasm", "execute", contract, json.dumps(fmsg(pr2, t2))))
    step("fwd_ackerr_claim_tx_ok", res.get("code") == 0, "note evm_%d %s" % (i2, res.get("raw_log", "")[:160]))
    ok = wait_for(lambda: rd.tbal(fb2) == pr2["amount"], 120)
    step("fwd_ack_error_refunds_fallback", ok and len(pending()) == 0, "fallback %s bal %d (want %d) pending %s" % (fb2, rd.tbal(fb2), pr2["amount"], pending()))
    R["claims"].append({"note": "evm_%d" % i2, "amount": pr2["amount"], "forward": True})

    # 3. timeout: relayer stopped, 60 s packet timeout, then relayer back -> MsgTimeout -> refund
    res = tx("owner", 400000, "wasm", "execute", contract, json.dumps({"set_forward_channel": {"channel_id": ch_a, "timeout_seconds": 60, "enabled": True}}))
    step("fwd_set_short_timeout", res.get("code") == 0, res.get("raw_log", "")[:120])
    hermes_stop()
    i3 = free.pop(0); fb3 = rd.new_wallet("fwd_fb3")
    t3 = tgt(ch_a, recv, fb3)
    pr3, _ = fprove(i3, t3)
    b3 = b_balances(recv)
    res = rec("fwd_claim_timeout", tx("stranger", G, "wasm", "execute", contract, json.dumps(fmsg(pr3, t3))))
    step("fwd_timeout_claim_tx_ok", res.get("code") == 0 and len(pending()) == 1, "note evm_%d pending %s %s" % (i3, pending(), res.get("raw_log", "")[:160]))
    # owner resolve is refused before timeout + grace
    p = pending()
    if p:
        rd.neg("fwd_resolve_before_grace", tx("owner", 400000, "wasm", "execute", contract,
               json.dumps({"resolve_forward": {"channel_id": ch_a, "sequence": p[0]["sequence"]}})), "grace")
    time.sleep(80)
    hermes_start()
    ok = wait_for(lambda: rd.tbal(fb3) == pr3["amount"], 180)
    step("fwd_timeout_refunds_fallback", ok and len(pending()) == 0 and b_balances(recv) == b3,
         "fallback %s bal %d (want %d) pending %s recv unchanged %s" % (fb3, rd.tbal(fb3), pr3["amount"], pending(), b_balances(recv) == b3))
    R["claims"].append({"note": "evm_%d" % i3, "amount": pr3["amount"], "forward": True})

    # 4. address reuse is NOT enforced on chain (frontend rule): same receiver + same fallback, new note -> accepted
    tx("owner", 400000, "wasm", "execute", contract, json.dumps({"set_forward_channel": {"channel_id": ch_a, "timeout_seconds": 600, "enabled": True}}))
    i4 = free.pop(0)
    pr4, _ = fprove(i4, t1)
    res = rec("fwd_claim_reuse", tx("stranger", G, "wasm", "execute", contract, json.dumps(fmsg(pr4, t1))))
    step("fwd_address_reuse_not_enforced_on_chain", res.get("code") == 0, "same receiver+fallback, note evm_%d %s" % (i4, res.get("raw_log", "")[:160]))
    wait_for(lambda: len(pending()) == 0, 60)
    R["claims"].append({"note": "evm_%d" % i4, "amount": pr4["amount"], "forward": True})
    hermes_stop()

if __name__ == "__main__":
    rd.FWD_HOOK = hook
    try:
        rd.main()
    except Exception as e:
        import traceback; traceback.print_exc(); step("harness_exception", False, repr(e)[:300])
    finally:
        hermes_stop()
        rd.save()
        p = sum(s["pass"] for s in R["steps"]); f = len(R["steps"]) - p
        rd.log("E2E_RESULT passed=%d failed=%d report=%s" % (p, f, os.path.join(rd.RUN, "report.json")))
        sys.exit(1 if f else 0)
