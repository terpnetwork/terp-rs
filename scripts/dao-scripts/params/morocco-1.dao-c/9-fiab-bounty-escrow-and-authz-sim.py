import json,base64,urllib.request,sys
L="http://127.0.0.1:11317"
FIAB="terp1nywjc2mfzl2fud8epzrllj2f2xlpnk2tfpnf49qh02z7c97gv2dq0sucwc"
PMOD="terp16aj9tt0lgz7pprcjxd3mre4gt7anwnrk55qehmm49wt8esaq4lwstkwl8z"
MEMBER="terp1fazvzt6pdpm4lg6t73h0j4dsyuc8tdxwrlwq9j"
OLD_ESCROW="terp1hpv9m7qwxde58pefcc0d42arrkdljlnzz3dskh8xna6z8z8vhkwq5hnchv"
OUT=sys.argv[1]
def get(p): return json.load(urllib.request.urlopen(L+p,timeout=15))
def vi(n):
    b=b""
    while True:
        x=n&0x7f; n>>=7
        if n: b+=bytes([x|0x80])
        else: return b+bytes([x])
def fb(f,v):
    if isinstance(v,str): v=v.encode()
    return vi(f<<3|2)+vi(len(v))+v
def fv(f,n): return vi(f<<3)+vi(n)
def anyb(t,v): return fb(1,t)+fb(2,v)
acct=get("/cosmos/auth/v1beta1/accounts/"+MEMBER)["account"]
pk=base64.b64decode(acct["pub_key"]["key"]); seq=int(acct["sequence"])
def simulate(msgs):
    body=b"".join(fb(1,m) for m in msgs)
    si=fb(1,anyb("/cosmos.crypto.secp256k1.PubKey",fb(1,pk)))+fb(2,fb(1,fv(1,1)))+fv(3,seq)
    auth=fb(1,si)+fb(2,b"")
    raw=fb(1,body)+fb(2,auth)+fb(3,b"\0"*64)
    req=urllib.request.Request(L+"/cosmos/tx/v1beta1/simulate",json.dumps({"tx_bytes":base64.b64encode(raw).decode()}).encode(),{"Content-Type":"application/json"})
    try: r=json.load(urllib.request.urlopen(req,timeout=30)); return {"ok":True,"gas_used":int(r["gas_info"]["gas_used"])}
    except urllib.error.HTTPError as e: return {"ok":False,"error":e.read().decode()[:600]}
def exec_msg(sender,contract,msg): return anyb("/cosmwasm.wasm.v1.MsgExecuteContract",fb(1,sender)+fb(2,contract)+fb(3,json.dumps(msg,separators=(",",":"))))
def b64j(o): return base64.b64encode(json.dumps(o,separators=(",",":")).encode()).decode()
# escrow init mirrors the live bounty escrow config, with FIAB as dao/owner/approver
cfg=get("/cosmwasm/wasm/v1/contract/%s/smart/%s"%(OLD_ESCROW,base64.b64encode(b'{"config":{}}').decode()))["data"]
init={"owner":FIAB,"approver":FIAB,"gate":{"use_allowlist":cfg["gate"]["use_allowlist"],"min_stake":cfg["gate"]["min_stake"],"cw4_group":None},
 "denoms":cfg["denoms"],"allow_partial_payouts":cfg["allow_partial_payouts"],"fee_bps":cfg["fee_bps"],"fee_collector":None,
 "min_duration_secs":cfg["min_duration_secs"],"max_duration_secs":cfg["max_duration_secs"],"params_version":cfg["params_version"],
 "params_placeholder":cfg["params_placeholder"],"min_submit_stake":cfg["min_submit_stake"],"min_upload_stake":cfg["min_upload_stake"],
 "initial_allowlist":[FIAB],"dao":FIAB,"pre_propose_info":{"anyone_may_propose":{}},"creation_gate":{"proposer_gate":{}}}
upm={"update_proposal_modules":{"to_add":[{"code_id":104,"msg":b64j(init),"admin":{"core_module":{}},"funds":[],"label":"fiab-bounty-escrow"}],"to_disable":[]}}
p1={"propose":{"title":"Add the bounty escrow proposal module to FIAB-SUBDAO",
 "description":"Instantiates a new bounty escrow (code 104) as a FIAB-SUBDAO proposal module through update_proposal_modules. FIAB-SUBDAO is the escrow dao, owner, approver and wasm admin, and it is on the initial allowlist so it can post bounties (gate: allowlist OR 1000 TERP bonded). The config (stake gates 1000/100/10 TERP, uterp/uthiol denom rules, durations, params_version) is copied from the live bounty DAO escrow "+OLD_ESCROW+". Nothing is migrated.",
 "msgs":[{"wasm":{"execute":{"contract_addr":FIAB,"msg":b64j(upm),"funds":[]}}}]}}
KEYS=["create_bounty","approve","reject","assign","unassign","request_changes"]
CAP=[("uterp","50000000000"),("uthiol","100000000000")] # 50k TERP + 100k THIOL total reward funds over the grant life
CALLS=200
EXP=1824854400 # 2027-10-29T00:00:00Z
def grant_bytes(granter,grantee,contract):
    lim=anyb("/cosmwasm.wasm.v1.CombinedLimit",fv(1,CALLS)+b"".join(fb(2,fb(1,d)+fb(2,a)) for d,a in CAP))
    flt=anyb("/cosmwasm.wasm.v1.AcceptedMessageKeysFilter",b"".join(fb(1,k) for k in KEYS))
    auth=anyb("/cosmwasm.wasm.v1.ContractExecutionAuthorization",fb(1,fb(1,contract)+fb(2,lim)+fb(3,flt)))
    return fb(1,granter)+fb(2,grantee)+fb(3,fb(1,auth)+fb(2,fv(1,EXP)))
def p2(contract):
    return {"propose":{"title":"Grant the FIAB member authz to post and curate bounties",
 "description":"MsgGrant from FIAB-SUBDAO to its member "+MEMBER+": ContractExecutionAuthorization on the FIAB bounty escrow "+contract+", limited to the top-level keys "+", ".join(KEYS)+", capped by CombinedLimit at 200 calls and 50,000 TERP + 100,000 THIOL total funds (the bounty rewards FIAB escrows via create_bounty; refunds return to FIAB as proposer). Expires 2027-10-29. create_bounty is sent as FIAB, so the reward is escrowed from the FIAB treasury and FIAB is the bounty proposer. Accept, payout, settle_remainder and cancel stay with FIAB proposals.",
 "msgs":[{"stargate":{"type_url":"/cosmos.authz.v1beta1.MsgGrant","value":base64.b64encode(grant_bytes(FIAB,MEMBER,contract)).decode()}}]}}
# ---- combined proposal 9 (instantiate2 via update_proposal_modules salt) ----
import hashlib
KEYS=["create_bounty","approve","reject","assign","unassign","request_changes","cancel"]
SALT=b"fiab-bounty-escrow-v1"
CHECKSUM=bytes.fromhex(get("/cosmwasm/wasm/v1/code-info/104")["checksum"])
CH="qpzry9x8gf2tvdw0s3jn54khce6mua7l"
def _pm(v):
    c=1
    for x in v:
        b=c>>25; c=(c&0x1ffffff)<<5^x
        for i in range(5): c^=[0x3b6a57b2,0x26508e6d,0x1ea119fa,0x3d4233dd,0x2a1462b3][i] if (b>>i)&1 else 0
    return c
def _hrp(h): return [ord(x)>>5 for x in h]+[0]+[ord(x)&31 for x in h]
def _conv(d,f,t,pad=True):
    a=0;bits=0;r=[];m=(1<<t)-1
    for v in d:
        a=(a<<f)|v;bits+=f
        while bits>=t: bits-=t;r.append((a>>bits)&m)
    if pad and bits: r.append((a<<(t-bits))&m)
    return r
def bech_dec(s):
    h,d=s.rsplit("1",1); return bytes(_conv([CH.index(c) for c in d[:-6]],5,8,False))
def bech_enc(h,b):
    d=_conv(b,8,5); p=_pm(_hrp(h)+d+[0]*6)^1
    return h+"1"+"".join(CH[x] for x in d+[(p>>5*(5-i))&31 for i in range(6)])
def u64(n): return n.to_bytes(8,"big")
def predict(creator,salt,checksum=CHECKSUM):
    c=bech_dec(creator)
    key=b"wasm\0"+u64(len(checksum))+checksum+u64(len(c))+c+u64(len(salt))+salt+u64(0)
    th=hashlib.sha256(b"module").digest()
    return bech_enc("terp",hashlib.sha256(th+key).digest())
PRED=predict(FIAB,SALT)
def build_addr(creator):
    try: return get("/cosmwasm/wasm/v1/contract/build_address?code_hash=%s&creator_address=%s&salt=%s"%(CHECKSUM.hex(),creator,SALT.hex())).get("address")
    except Exception as e: return "n/a: %s"%e
def exists(a):
    try: get("/cosmwasm/wasm/v1/contract/"+a); return True
    except Exception: return False
upm2={"update_proposal_modules":{"to_add":[{"code_id":104,"msg":b64j(init),"admin":{"core_module":{}},"funds":[],"label":"fiab-bounty-escrow","salt":base64.b64encode(SALT).decode()}],"to_disable":[]}}
grant_b64=base64.b64encode(grant_bytes(FIAB,MEMBER,PRED)).decode()
p9={"propose":{"title":"Add the FIAB bounty escrow module (instantiate2) and grant the member curation authz",
 "description":"Msg 1: update_proposal_modules instantiates a new bounty escrow (code 104, checksum "+CHECKSUM.hex()+") as a FIAB-SUBDAO proposal module via instantiate2, salt \"fiab-bounty-escrow-v1\", creator FIAB-SUBDAO, wasm admin FIAB-SUBDAO (core_module). Deterministic address: "+PRED+". FIAB-SUBDAO is the escrow dao, owner, approver and only initial allowlist entry; config copied from the live bounty DAO escrow "+OLD_ESCROW+". Nothing is migrated. Msg 2: MsgGrant from FIAB-SUBDAO to its member "+MEMBER+": ContractExecutionAuthorization on "+PRED+" only, top-level keys "+", ".join(KEYS)+", CombinedLimit 200 calls and 50,000 TERP + 100,000 THIOL total funds, expiring 2027-10-29. create_bounty is sent as FIAB, so rewards come from the FIAB treasury and refunds return to FIAB. Accept, payout and settle_remainder stay with FIAB proposals.",
 "msgs":[{"wasm":{"execute":{"contract_addr":FIAB,"msg":b64j(upm2),"funds":[]}}},
         {"stargate":{"type_url":"/cosmos.authz.v1beta1.MsgGrant","value":grant_b64}}]}}
def sim_ev(msgs):
    body=b"".join(fb(1,m) for m in msgs)
    si=fb(1,anyb("/cosmos.crypto.secp256k1.PubKey",fb(1,pk)))+fb(2,fb(1,fv(1,1)))+fv(3,seq)
    raw=fb(1,body)+fb(2,fb(1,si)+fb(2,b""))+fb(3,b"\0"*64)
    req=urllib.request.Request(L+"/cosmos/tx/v1beta1/simulate",json.dumps({"tx_bytes":base64.b64encode(raw).decode()}).encode(),{"Content-Type":"application/json"})
    try:
        r=json.load(urllib.request.urlopen(req,timeout=30))
        addrs=[a["value"] for e in r["result"]["events"] for a in e["attributes"] if a["key"]=="_contract_address"]
        return {"ok":True,"gas_used":int(r["gas_info"]["gas_used"]),"contract_addresses":sorted(set(addrs))}
    except urllib.error.HTTPError as e: return {"ok":False,"error":e.read().decode()[:500]}
inst2=lambda sender: anyb("/cosmwasm.wasm.v1.MsgInstantiateContract2",fb(1,sender)+fb(2,FIAB)+fv(3,104)+fb(4,"fiab-bounty-escrow")+fb(5,json.dumps(init,separators=(",",":")))+fb(7,SALT))
res={}
res["S5_propose_combined_9"]=simulate([exec_msg(MEMBER,PMOD,p9)])
res["S6_parse_check_upm_with_salt_as_member"]=simulate([exec_msg(MEMBER,FIAB,upm2)])
res["S7_proxy_instantiate2_by_member"]=sim_ev([inst2(MEMBER)])
res["S7_expected_member_addr"]=predict(MEMBER,SALT)
res["S8_grant_shape_member_to_pred"]=simulate([anyb("/cosmos.authz.v1beta1.MsgGrant",grant_bytes(MEMBER,FIAB,PRED))])
res["predicted"]=PRED; res["build_address_rest_fiab"]=build_addr(FIAB); res["build_address_rest_member"]=build_addr(MEMBER)
res["predicted_already_exists"]=exists(PRED)
out=dict({"dao":FIAB,"proposal_module":PMOD,"signer":MEMBER,"status":"DRAFT - HOLD, do not broadcast on mainnet","creation_policy":"anyone (no pre-propose, no deposit)","flow":"threshold 1, no veto: propose, vote yes, execute (sole member)",
 "instantiate2":{"route":"dao-dao-core update_proposal_modules ModuleInstantiateInfo.salt (supported since dao-contracts v2.7.0 #888; deployed core is 2.8.0-alpha.2) -> core emits WasmMsg::Instantiate2 and registers the module in reply","creator":FIAB,"code_id":104,"checksum":CHECKSUM.hex(),"salt_utf8":SALT.decode(),"salt_b64":base64.b64encode(SALT).decode(),"fix_msg":False,"predicted_address":PRED,"rest_build_address":res["build_address_rest_fiab"]},
 "escrow_instantiate_msg":init,"update_proposal_modules":upm2,
 "authz":{"granter":FIAB,"grantee":MEMBER,"contract":PRED,"limit":"CombinedLimit 200 calls; 50000000000uterp + 100000000000uthiol","filter":{"accepted_message_keys":KEYS},"expiration":"2027-10-29T00:00:00Z"},
 "simulation":res},propose=p9,vote={"vote":{"proposal_id":"<next id>","vote":"yes"}},execute={"execute":{"proposal_id":"<next id>"}})
json.dump(out,open(OUT+"/9-fiab-bounty-escrow-and-authz.json","w"),indent=2)
print(json.dumps(res,indent=1))
