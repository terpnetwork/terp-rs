#!/usr/bin/env python3
"""Headstash gov signing session: the exec + revoke half of the cw-orch-kms session.

The grant half (one small amino doc for Zigner) comes from gov-session/ (bin gov-session-grant,
built on cw-orch-kms plan_session_setup). This module builds the UNSIGNED txs the on-device
ephemeral key signs afterwards:
  exec   MsgExec{grantee=eph, msgs=[MsgSubmitProposal...]}, fee paid by the granter's feegrant
  revoke MsgExec{MsgRevoke(type) for each session type, MsgRevokeAllowance, MsgRevoke(MsgRevoke) last}
Nothing here signs or broadcasts. Pure JSON in, JSON out.

CLI:
  gov_session.py exec   --grantee EPH --granter G --inner unsigned_submit.json [--gas N --fee N --denom uthiol] -o out.json
  gov_session.py revoke --grantee EPH --granter G [--type URL]... [--gas N --fee N --denom uthiol] -o out.json
"""
import argparse, json, sys

SUBMIT = "/cosmos.gov.v1.MsgSubmitProposal"
REVOKE = "/cosmos.authz.v1beta1.MsgRevoke"
REVOKE_ALLOWANCE = "/cosmos.feegrant.v1beta1.MsgRevokeAllowance"
SESSION_TYPES = (SUBMIT, REVOKE, REVOKE_ALLOWANCE)  # local dry run set


def _tx(msgs, gas, fee, denom, granter, memo=""):
    return {"body": {"messages": msgs, "memo": memo, "timeout_height": "0", "extension_options": [],
                     "non_critical_extension_options": []},
            "auth_info": {"signer_infos": [], "fee": {"amount": [{"denom": denom, "amount": str(fee)}], "gas_limit": str(gas),
                                                      "payer": "", "granter": granter or ""}, "tip": None},
            "signatures": []}


def exec_tx(grantee, granter, inner_msgs, gas, fee, denom, memo="headstash-gov-session:exec"):
    for m in inner_msgs:
        if m.get("@type") not in SESSION_TYPES:
            raise ValueError("inner msg %s not in session grant" % m.get("@type"))
        if m.get("@type") == SUBMIT and m.get("proposer") != granter:
            raise ValueError("proposer %s != granter %s" % (m.get("proposer"), granter))
    return _tx([{"@type": "/cosmos.authz.v1beta1.MsgExec", "grantee": grantee, "msgs": inner_msgs}], gas, fee, denom, granter, memo)


def revoke_msgs(granter, grantee, types=SESSION_TYPES):
    """Order: plain types, then MsgRevokeAllowance (needs its grant still live), its grant, MsgRevoke authority last."""
    r = lambda t: {"@type": REVOKE, "granter": granter, "grantee": grantee, "msg_type_url": t}
    out = [r(t) for t in types if t not in (REVOKE, REVOKE_ALLOWANCE)]
    if REVOKE_ALLOWANCE in types:
        out += [{"@type": REVOKE_ALLOWANCE, "granter": granter, "grantee": grantee}, r(REVOKE_ALLOWANCE)]
    if REVOKE in types:
        out.append(r(REVOKE))
    return out


def revoke_tx(grantee, granter, types, gas, fee, denom, use_feegrant=False):
    # fee paid by the eph key's feegrant from granter only if it still exists at ante time (it does: revoke runs after)
    return _tx([{"@type": "/cosmos.authz.v1beta1.MsgExec", "grantee": grantee, "msgs": revoke_msgs(granter, grantee, types)}],
               gas, fee, denom, granter if use_feegrant else "", "headstash-gov-session:revoke")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["exec", "revoke"])
    ap.add_argument("--grantee", required=True); ap.add_argument("--granter", required=True)
    ap.add_argument("--inner"); ap.add_argument("--type", action="append")
    ap.add_argument("--gas", type=int, default=0); ap.add_argument("--fee", type=int, default=0)
    ap.add_argument("--denom", default="uthiol"); ap.add_argument("-o", "--out", required=True)
    a = ap.parse_args()
    if a.cmd == "exec":
        inner = json.load(open(a.inner))
        msgs = inner["body"]["messages"] if "body" in inner else inner["messages"]
        t = exec_tx(a.grantee, a.granter, msgs, a.gas or 6000000, a.fee or 150000000, a.denom)
    else:
        t = revoke_tx(a.grantee, a.granter, tuple(a.type or SESSION_TYPES), a.gas or 400000, a.fee or 20000, a.denom, use_feegrant=True)
    s = json.dumps(t, separators=(",", ":"))
    open(a.out, "w").write(s)
    print(json.dumps({"out": a.out, "bytes": len(s), "msgs": len(t["body"]["messages"][0]["msgs"]), "signed": False}))


if __name__ == "__main__":
    main()

# The wasm store is NOT an authz session type: cw-orch-kms renders MsgStoreCode scope as a
# StoreCodeAuthorization with CodeGrant.instantiate_permission = None (the Accept nil panic), and the
# gzipped wasm (~281 KB) is over Zigner 64 KiB. The ephemeral key stores it as itself in a normal tx
# (sender = eph, instantiate_permission = gov only), fee paid by the session feegrant.
SESSION_TYPES_MAINNET = SESSION_TYPES
# The frontend wasm (cw_headstash_frontend.wasm, 13b) is stored the same way. Frontend chunk uploads are permissionless and
# content-addressed, so they need no grant at all: a throwaway uploader key signs them, fees granted directly by terp1fx
# (render_gov_session.py 18-20). The session grant therefore stays exactly SESSION_TYPES.
