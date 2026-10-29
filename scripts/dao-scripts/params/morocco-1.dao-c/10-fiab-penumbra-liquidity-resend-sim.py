# simulate proposal 10 (FIAB prop 5) propose by the member; check FIAB balance
import json,base64,urllib.request,sys
L="http://127.0.0.1:11317"
P=json.load(open(sys.argv[1] if len(sys.argv)>1 else "10-fiab-penumbra-liquidity-resend.json"))
FIAB,PMOD,MEMBER=P["dao"],P["proposal_module"],P["signer"]
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
    raw=fb(1,body)+fb(2,fb(1,si)+fb(2,b""))+fb(3,b"\0"*64)
    req=urllib.request.Request(L+"/cosmos/tx/v1beta1/simulate",json.dumps({"tx_bytes":base64.b64encode(raw).decode()}).encode(),{"Content-Type":"application/json"})
    try: r=json.load(urllib.request.urlopen(req,timeout=30)); return {"ok":True,"gas_used":int(r["gas_info"]["gas_used"])}
    except urllib.error.HTTPError as e: return {"ok":False,"error":e.read().decode()[:600]}
def ex(msg): return anyb("/cosmwasm.wasm.v1.MsgExecuteContract",fb(1,MEMBER)+fb(2,PMOD)+fb(3,json.dumps(msg,separators=(",",":"))))
q=lambda m: get("/cosmwasm/wasm/v1/contract/%s/smart/%s"%(PMOD,base64.b64encode(json.dumps(m).encode()).decode()))["data"]
nxt=max([p["id"] for p in q({"list_proposals":{"limit":50}})["proposals"]]+[0])+1
bal={b["denom"]:int(b["amount"]) for b in get("/cosmos/bank/v1beta1/balances/"+FIAB)["balances"]}
need={}
for m in P["propose"]["propose"]["msgs"]:
    t=m["ibc"]["transfer"]; need[t["amount"]["denom"]]=need.get(t["amount"]["denom"],0)+int(t["amount"]["amount"])
print(json.dumps({"next_proposal_id":nxt,"json_proposal_id":P["proposal_id"],"fiab_balance":bal,"need":need,
 "covered":all(bal.get(d,0)>=a for d,a in need.items()),"propose_sim":simulate([ex(P["propose"])])},indent=1))
