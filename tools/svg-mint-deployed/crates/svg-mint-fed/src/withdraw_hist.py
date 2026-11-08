"""Admin withdraw history from the house wallet. Prints JSON only."""
import json
import os
import sqlite3
import sys
import urllib.request
import base64

ua = sys.argv[1] if len(sys.argv) > 1 else ""
uas = [ua] if ua.startswith("u1") else []
limit = 20
before = None
if len(sys.argv) > 2 and sys.argv[2].isdigit():
    limit = max(1, min(50, int(sys.argv[2])))
if len(sys.argv) > 3 and sys.argv[3].isdigit():
    before = int(sys.argv[3])
db = os.path.join(os.environ.get("ZCASH_DEVTOOL_WALLET", ""), "data.sqlite")
if not uas or not os.path.isfile(db):
    print(json.dumps({"ok": True, "withdraws": [], "next": None}))
    raise SystemExit(0)

con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
cur = con.cursor()
q = """
SELECT t.id_tx, hex(t.txid), t.mined_height, t.expiry_height, t.fee, t.created, s.value, length(t.raw)
FROM sent_notes s
JOIN transactions t ON t.id_tx = s.transaction_id
WHERE s.to_address = ?
"""
args = [uas[0]]
if before is not None:
    q += " AND t.id_tx < ?"
    args.append(before)
q += " ORDER BY t.id_tx DESC LIMIT ?"
args.append(limit + 1)
rows = list(cur.execute(q, args))
more = len(rows) > limit
rows = rows[:limit]

cookie_path = os.environ.get("ZCASH_PRIVATE_RPC_COOKIE", "")
url = os.environ.get("ZCASH_PRIVATE_RPC", "http://127.0.0.1:8232")
cookie = ""
if cookie_path and os.path.isfile(cookie_path):
    cookie = open(cookie_path).read().strip()
auth = base64.b64encode(cookie.encode()).decode() if cookie else ""
tip = None

def rpc(method, params):
    if not auth:
        return None
    req = urllib.request.Request(
        url,
        data=json.dumps({"jsonrpc": "1.0", "id": "w", "method": method, "params": params}).encode(),
        headers={"Authorization": "Basic " + auth, "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=8) as r:
            body = json.load(r)
        return body.get("result")
    except Exception:
        return None

if auth:
    tip = rpc("getblockcount", [])

out = []
for id_tx, txhex, mined, expiry, fee, created, value, raw_len in rows:
    raw = bytes.fromhex(txhex)
    disp = raw[::-1].hex()
    on_chain = None
    if auth:
        on_chain = rpc("getrawtransaction", [disp, 1])
    height = mined
    if isinstance(on_chain, dict) and on_chain.get("height"):
        height = on_chain.get("height")
    if height:
        status = "mined"
        note = "In a block. The grant wallet shows it after it scans this height."
    elif expiry and tip and int(expiry) < int(tip):
        status = "expired"
        note = "Not in the grant wallet. The node dropped it before it was mined."
    elif isinstance(on_chain, dict):
        status = "mempool"
        note = "In the node mempool. Not confirmed yet."
    else:
        status = "missing"
        note = "The house wallet has this send, but the node does not."
    if status == "expired" and fee == 10000 and raw_len and raw_len > 2000:
        note = "Not in the grant wallet. Fee was 10000 zat on a %s-byte transaction, below what the node will keep, so it expired." % raw_len
    out.append({
        "id": id_tx,
        "at": created,
        "zat": value,
        "fee": fee,
        "expiry": expiry,
        "height": height,
        "txid": disp,
        "status": status,
        "note": note,
    })
next_id = out[-1]["id"] if more and out else None
print(json.dumps({"ok": True, "withdraws": out, "next": next_id, "tip": tip}))
