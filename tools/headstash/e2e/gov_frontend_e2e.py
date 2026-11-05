#!/usr/bin/env python3
"""Mainnet gov-session dry run with the on-chain claim frontend (local chain only, loopback only).

run_direct.py with GOV_ROLES=1 plus FE_GOV: the ephemeral session key stores cw_headstash.wasm and
cw_headstash_frontend.wasm (sender = eph, instantiate permission gov only, fee via the session feegrant),
the session is revoked, then ONE proposal by the proposer: CommunityPoolSpend, Instantiate2 frontend
(gov admin + owner, predicted address), PublishManifest (activate), Instantiate2 cw-headstash with
frontend_contract = predicted frontend address, set campaign, bind circuit. After it passes, an
uploader key fee-granted by the terp1fx stand-in (relayer key) uploads the content-addressed chunks;
the last chunk seals the version and makes it current. Then browser_e2e loads the page from chain
(every hash checked), registers and claims every note. Mirrors deploy/render_gov_session.py.
"""
import json, os, time, base64
os.environ["GOV_ROLES"] = "1"
import run_direct as rd
import browser_e2e as bf
import frontend_bench as fb
from run import tx, smart, gas_used

GOV = rd.GOV
R = rd.R

def manifest():
    m, _ = fb.manifest(bf.page_files(), bf.CHUNK); m["entry"] = "app.js"; m["note"] = "headstash claim page"; m["activate"] = True
    return m

rd.FE_GOV = {"wasm": fb.FE_WASM, "manifest": manifest, "max_chunk_bytes": bf.CHUNK}

def publish(contract):
    fe = R["fe_contract"]; relayer = R["addresses"]["RELAYER_ADDR"]
    ci = rd.q("wasm", "contract", fe); ci = ci.get("contract_info", ci)
    own = smart(fe, {"ownership": {}})
    rd.step("fe_instantiated_by_gov", ci.get("admin") == GOV and ci.get("creator") == GOV and own.get("owner") == GOV and str(ci.get("code_id")) == str(R["fe_code_id"]),
            "%s admin %s creator %s owner %s code %s" % (fe, ci.get("admin"), ci.get("creator"), own.get("owner"), ci.get("code_id")))
    rd.step("headstash_frontend_contract_predicted", smart(contract, {"frontend_contract": {}}) == fe, "frontend_contract %s" % fe)
    m_on = smart(fe, {"manifest": {"version": 1}})
    rd.step("fe_manifest_published_by_gov", m_on == R["fe_manifest"], "v1 %d files %d chunks" % (len(m_on.get("files", [])), sum(len(f["chunks"]) for f in m_on.get("files", []))))
    cfg = smart(fe, {"config": {}}); rd.step("fe_not_current_before_upload", cfg.get("current") is None, json.dumps(cfg))
    rd.neg("fe_publish_by_non_gov_refused", tx("owner", 3000000, "wasm", "execute", fe, json.dumps({"publish_manifest": {"manifest": dict(R["fe_manifest"], version=2)}})))
    # uploader: throwaway key, fees granted by the terp1fx stand-in (BasicAllowance, capped, 1h expiry)
    up = rd.new_wallet("fe_uploader"); R["addresses"]["FE_UPLOADER"] = up
    exp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() + 3600))
    cap = 0
    _, chunks = fb.manifest(bf.page_files(), bf.CHUNK)
    gas_of = lambda c: 600000 + 64 * len(c)
    cap = sum(int(gas_of(c) * 0.025) + 1 for cs in chunks for c in cs) + 1000000
    res = tx("relayer", 200000, "feegrant", "grant", relayer, up, "--spend-limit", "%duterp" % cap, "--expiration", exp)
    rd.step("fe_uploader_feegrant_from_fx", res.get("code") == 0, "cap %d uterp exp %s %s" % (cap, exp, res.get("raw_log", "")[:120]))
    bad = bytearray(chunks[0][0]); bad[0] ^= 1
    rd.neg("fe_tampered_chunk_refused", tx("fe_uploader", gas_of(bad), "wasm", "execute", fe, json.dumps({"upload_chunk": {"version": 1, "file": 0, "index": 0, "data": base64.b64encode(bytes(bad)).decode()}}), "--fee-granter", relayer))
    n, gas = 0, []
    for fi, cs in enumerate(chunks):
        for ci_, c in enumerate(cs):
            res = tx("fe_uploader", gas_of(c), "wasm", "execute", fe, json.dumps({"upload_chunk": {"version": 1, "file": fi, "index": ci_, "data": base64.b64encode(c).decode()}}), "--fee-granter", relayer)
            n += res.get("code") == 0; gas.append([len(c), gas_used(res), res.get("code")])
    R["fe_upload_gas"] = gas
    st = smart(fe, {"status": {"version": 1}}); cfg = smart(fe, {"config": {}})
    rd.step("fe_uploaded_sealed_current", st.get("complete") is True and cfg.get("current") == 1 and n == len(gas),
            "%d/%d chunk txs, %s, config %s, uploader balance %d" % (n, len(gas), json.dumps(st), json.dumps(cfg), bf.bal(up)))
    res = tx("relayer", 200000, "feegrant", "revoke", relayer, up)
    rd.step("fe_uploader_feegrant_revoked", res.get("code") == 0, res.get("raw_log", "")[:120])
    wavs_negatives(contract)
    fs = bf.page_files()
    R["page"] = {"frontend": fe, "files": [{"path": f["path"], "raw": len(f["raw"]), "gzip": len(f["stored"]), "sha256": bf.hashlib.sha256(f["raw"]).hexdigest()} for f in fs],
                 "total_gzip": sum(len(f["stored"]) for f in fs), "chunks": len(gas)}
    return fe

def wavs_negatives(contract):
    """WAVS is disabled (wavs = null): the contract reports it, stores no operator set, and has
    no execute path to set one. The smart-account authenticator handlers were removed and the
    sudo entry point only decodes HashMerchantSudoMsg, so attaching the contract as an
    authenticator is refused on chain."""
    if os.environ.get("WAVS_DISABLED") == "1":
        st = smart(contract, {"wavs_status": {}})
        rd.step("wavs_status_disabled", st.get("enabled") is False and st.get("operator_set") is None, json.dumps(st))
        cfg_has = R.get("wavs_aggregate_key")
        rd.step("wavs_instantiated_with_null", cfg_has is None, "instantiate msg wavs = null")
    elif os.environ.get("WAVS_JSON"):
        dec = json.load(open(os.environ["WAVS_JSON"]))["msg"]["aggregate_key"]
        rd.step("wavs_set_is_rendered_mainnet_set", R.get("wavs_aggregate_key") == dec, "aggregate_key %s..." % dec[:16])
    init = json.dumps({"contract": contract, "params": base64.b64encode(b"{}").decode()}).encode()
    data = base64.b64encode(init).decode()  # the CLI base64-decodes [data] into MsgAddAuthenticator.Data
    rd.neg("wavs_contract_as_smartaccount_authenticator_refused", tx("owner", 3000000, "smartaccount", "add-authenticator", "CosmwasmAuthenticatorV1", data),
           "OnAuthenticatorAdded", "hash_merchant")  # chain-side refusal, not a CLI error
    rd.neg("wavs_unknown_execute_refused", tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"wavs_rotate": {"nonce": 1}})))
    rd.neg("wavs_set_operator_execute_refused", tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"set_wavs": {"wavs": {}}})))

bf.publish = publish
if __name__ == "__main__":
    rd.main()
