#!/usr/bin/env python3
"""Continue a browser_e2e run when the local chain cannot fund every note at once.

The 43-note EVM test identity is owed 200008 TERP, more than this local chain's whole liquid
supply. The page claims in rounds: before each round, the test tops up the contract from
owner/proposer; between rounds it returns already-paid uterp from the claim wallets (keys
derived from the test notes file, imported into the throwaway test keyring) back to the contract.
Every claim is still made by the real page (Playwright, prover in the browser, fee grant via relay).
Local test chain only.
"""
import hashlib, hmac, json, os, subprocess, sys, time
import browser_e2e as be
import run_direct as rd
from run import tx, terpd, HOME, REST, RUN, RELAY_BIN, RELAY_BIND, CHAIN_ID, TERPD

N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
def hkdf(ikm, salt, info, L):
    prk = hmac.new(salt, ikm, hashlib.sha256).digest(); t = b""; out = b""; i = 1
    while len(out) < L: t = hmac.new(prk, t + info + bytes([i]), hashlib.sha256).digest(); out += t; i += 1
    return out[:L]
def claim_priv(s_hex): return (int.from_bytes(hkdf(bytes.fromhex(s_hex), b"terp-hs-claim-key-v1", b"", 32), "big") % (N - 1)) + 1

def main(work, contract, url_port=be.PORT):
    env = dict(os.environ, HEADSTASH_RELAY_HOME=HOME, HEADSTASH_RELAY_FROM="relayer", HEADSTASH_KEYRING_BACKEND="test",
               HEADSTASH_REST=REST, HEADSTASH_CONTRACT=contract, HEADSTASH_CHAIN_ID=CHAIN_ID, HEADSTASH_RELAY_BIND=RELAY_BIND,
               HEADSTASH_GAS_PRICE="0.025uterp", HEADSTASH_CLAIM_GAS=str(rd.CLAIM_GAS), HEADSTASH_REGISTER_GAS=str(rd.REGISTER_GAS),
               TERPD=TERPD, PATH="/usr/local/bin:" + os.environ.get("PATH", "")); env.pop("HEADSTASH_ELIGIBILITY", None)
    rl = os.path.join(work, "relay-rounds.log"); relay = subprocess.Popen([RELAY_BIN], env=env, stdout=open(rl, "w"), stderr=subprocess.STDOUT)
    for _ in range(60):
        time.sleep(1)
        if "relay_up" in open(rl).read(): break
    else: relay.kill(); raise SystemExit("relay did not come up: " + open(rl).read()[-300:])
    srv = be.serve(work); url = "http://127.0.0.1:%d/loader.html" % url_port
    cur = os.path.join(work, "notes-after-claim.json"); rounds = []; recycled = 0
    try:
        for r in range(1, 12):
            ok, nf = be.canon_ok(cur); left = [n for n in nf["notes"] if n["st"] != "spent"]
            if not left: break
            # return paid-out uterp from spent claim wallets (test-chain liquidity only)
            for n in nf["notes"]:
                if n["st"] != "spent": continue
                b = be.bal(n["dest"]); 
                if b <= 10000: continue
                name = "hsfe_cw_%d" % n["i"]; pk = "%064x" % claim_priv(n["s"])
                terpd("keys", "delete", name, "-y", "--keyring-backend", "test", "--home", HOME, check=False)
                terpd("keys", "import-hex", name, pk, "--keyring-backend", "test", "--home", HOME)
                assert terpd("keys", "show", name, "-a", "--keyring-backend", "test", "--home", HOME).strip() == n["dest"], "derived claim wallet mismatch"
                res = tx(name, 100000, "bank", "send", n["dest"], contract, "%duterp" % (b - 2500))
                recycled += (b - 2500) if res.get("code") == 0 else 0
                terpd("keys", "delete", name, "-y", "--keyring-backend", "test", "--home", HOME, check=False)
            cb = be.bal(contract); print("round", r, "left", len(left), "contract", cb, flush=True)
            out = os.path.join(work, "notes-round-%d.json" % r)
            b = be.browser("claim", {"url": url, "notes": cur, "out": out, "n": len(left)})
            ok2, nf2 = be.canon_ok(out) if b.get("saved") else (False, None)
            spent = sum(n["st"] == "spent" for n in nf2["notes"]) if nf2 else 0
            rounds.append({"round": r, "contract_before": cb, "spent_after": spent, "claim_ms": b.get("claim_ms"), "checksum_ok": ok2})
            print(json.dumps(rounds[-1]), flush=True)
            if not nf2 or spent == sum(n["st"] == "spent" for n in nf["notes"]): break
            cur = out
        ok, nf = be.canon_ok(cur)
        res = {"rounds": rounds, "recycled_uterp": recycled, "final_file": cur, "checksum_ok": ok,
               "spent": sum(n["st"] == "spent" for n in nf["notes"]), "total": len(nf["notes"]),
               "claimed_uterp_sum": sum(int(n["amt"]) for n in nf["notes"] if n["st"] == "spent"),
               "prove_ms": [n.get("prove_ms") for n in nf["notes"]]}
        # every nullifier spent on chain, independent of the page's own bookkeeping
        res["onchain_nullifiers_spent"] = sum(bool(be.smart(contract, {"nullifier_spent": {"nullifier": be.base64.b64encode(bytes.fromhex(n["n"])).decode()}}) ) for n in nf["notes"]) if "--q" in sys.argv else None
        json.dump(res, open("/tmp/hsfe/browser-claim-rounds.json", "w"), indent=1); print("RESULT", json.dumps({k: v for k, v in res.items() if k != "prove_ms"}))
    finally: srv.shutdown(); relay.terminate()

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
