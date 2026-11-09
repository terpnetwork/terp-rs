#!/usr/bin/env python3
"""Read-only scan of Headstash snapshot addresses: eth_getCode (Ethereum mainnet)
and Solana getMultipleAccounts. Writes JSON cache; no keys, no transactions."""
import json, os, sys, time, urllib.request, glob
ROOT = sys.argv[1]; OUT = sys.argv[2]
EVM_RPC = os.environ.get('EVM_RPC', 'https://ethereum-rpc.publicnode.com')
SOL_RPC = os.environ.get('SOL_RPC', 'https://api.mainnet-beta.solana.com')
def post(url, body):
    for i in range(6):
        try:
            req = urllib.request.Request(url, json.dumps(body).encode(), {'Content-Type': 'application/json', 'User-Agent': 'headstash-scan'})
            return json.load(urllib.request.urlopen(req, timeout=60))
        except Exception as e:
            print('retry', i, e, file=sys.stderr); time.sleep(2 + 3 * i)
    raise SystemExit('rpc failed: ' + url)
evm, sol = set(), set()
for d in sorted(glob.glob(ROOT + '/target/eligibility-stage/*')):
    n = os.path.basename(d)
    for line in open(f'{d}/{n}.csv').read().splitlines()[1:]:
        a = line.split(',')[0].strip()
        if not a: continue
        (evm.add(a.lower()) if a.lower().startswith('0x') else sol.add(a))
cache = json.load(open(OUT)) if os.path.exists(OUT) else {'evm': {}, 'sol': {}}
todo = sorted(a for a in evm if a not in cache['evm'])
for i in range(0, len(todo), 50):
    chunk = todo[i:i+50]
    res = post(EVM_RPC, [{'jsonrpc': '2.0', 'id': j, 'method': 'eth_getCode', 'params': [a, 'latest']} for j, a in enumerate(chunk)])
    byid = {r['id']: r for r in res}
    for j, a in enumerate(chunk):
        r = byid.get(j, {})
        if 'result' in r: cache['evm'][a] = r['result']
    if i % 1000 == 0:
        json.dump(cache, open(OUT, 'w')); print('evm', i, len(todo), file=sys.stderr)
    time.sleep(0.15)
json.dump(cache, open(OUT, 'w'))
todo = sorted(a for a in sol if a not in cache['sol'])
for i in range(0, len(todo), 100):
    chunk = todo[i:i+100]
    r = post(SOL_RPC, {'jsonrpc': '2.0', 'id': 1, 'method': 'getMultipleAccounts', 'params': [chunk, {'encoding': 'base64', 'dataSlice': {'offset': 0, 'length': 0}}]})
    for a, v in zip(chunk, r['result']['value']):
        cache['sol'][a] = None if v is None else {'owner': v['owner'], 'executable': v['executable'], 'lamports': v['lamports']}
    time.sleep(1.0)
json.dump(cache, open(OUT, 'w'))
print('evm', len(evm), 'cached', len(cache['evm']), 'sol', len(sol), 'cached', len(cache['sol']))
