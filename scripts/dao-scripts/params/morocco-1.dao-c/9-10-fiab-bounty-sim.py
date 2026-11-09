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
res={}
res["S1_propose_add_module"]=simulate([exec_msg(MEMBER,PMOD,p1)])
res["S2_escrow_instantiate_direct"]=simulate([anyb("/cosmwasm.wasm.v1.MsgInstantiateContract",fb(1,MEMBER)+fb(2,FIAB)+fv(3,104)+fb(4,"fiab-bounty-escrow-sim")+fb(5,json.dumps(init)))])
res["S3_grant_shape_member_to_fiab"]=simulate([anyb("/cosmos.authz.v1beta1.MsgGrant",grant_bytes(MEMBER,FIAB,OLD_ESCROW))])
res["S4_propose_authz"]=simulate([exec_msg(MEMBER,PMOD,p2(OLD_ESCROW))])
wrap=lambda title,prop,extra: dict({"dao":FIAB,"proposal_module":PMOD,"signer":MEMBER,"creation_policy":"anyone (no pre-propose, no deposit)","flow":"threshold 1, no veto: propose, vote yes, execute (sole member)","status":"DRAFT - not broadcast"},**extra,propose=prop,vote={"vote":{"proposal_id":"<next id>","vote":"yes"}},execute={"execute":{"proposal_id":"<next id>"}})
json.dump(wrap("",p1,{"escrow_instantiate_msg":init,"update_proposal_modules":upm,"simulation":{k:res[k] for k in ["S1_propose_add_module","S2_escrow_instantiate_direct"]}}),open(OUT+"/9-fiab-bounty-escrow-module.json","w"),indent=2)
t=p2("<FIAB_ESCROW_ADDRESS from prop 9 execute tx>"); t["propose"]["msgs"][0]["stargate"]["value"]="<regenerate with the real escrow address: python3 sim.py>"
json.dump(wrap("",t,{"authz":{"granter":FIAB,"grantee":MEMBER,"type":"/cosmwasm.wasm.v1.ContractExecutionAuthorization","contract":"<FIAB_ESCROW_ADDRESS>","limit":"CombinedLimit: 200 calls; funds cap 50000000000uterp + 100000000000uthiol total (decision: user to confirm)","filter":{"accepted_message_keys":KEYS},"expiration":"2027-10-29T00:00:00Z"},"simulation":{k:res[k] for k in ["S3_grant_shape_member_to_fiab","S4_propose_authz"]}}),open(OUT+"/10-fiab-member-curation-authz.json","w"),indent=2)
print(json.dumps(res,indent=1))
