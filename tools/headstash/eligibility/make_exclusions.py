"""Build eligibility/exclusions.csv and review.csv from the read-only scan cache (scan_addresses.py)."""
import json, glob, os, collections
R=os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # tools/headstash
c=json.load(open(os.environ.get('SCAN_CACHE', R+'/target/address-scan.json')))
SYS='11111111111111111111111111111111'
rows=collections.defaultdict(list)  # addr -> [(community, amount)]
for d in sorted(glob.glob(R+'/target/eligibility-stage/*')):
    n=os.path.basename(d)
    for l in open(f'{d}/{n}.csv').read().splitlines()[1:]:
        p=l.split(',');
        if len(p)<2 or not p[0].strip(): continue
        a=p[0].strip(); a=a.lower() if a.lower().startswith('0x') else a
        rows[a].append((n,p[1].strip()))
BURN={'0x0000000000000000000000000000000000000000':'zero address','0x000000000000000000000000000000000000dead':'burn sink 0x...dEaD',
      '0xdead000000000000000000000000000000000000':'burn sink 0xdEaD00...','0xdead000000000000000042069420694206942069':'burn sink (Optimism-style)'}
SOLBURN={'1nc1nerator11111111111111111111111111111111':'Solana incinerator',SYS:'Solana System Program'}
ex=[]; rv=[]
for a,code in sorted(c['evm'].items()):
    if a in BURN:
        for comm,amt in rows[a]: ex.append((a,comm,'burn',BURN[a]+'; snapshot amount '+amt))
    elif code not in ('0x','',None) and not code.startswith('0xef0100'):
        safe='a619486e' in code[:200]
        for comm,amt in rows[a]:
            t=(a,comm,'safe-multisig' if safe else 'contract',f'eth_getCode(latest) on Ethereum mainnet: {(len(code)-2)//2} bytes'+(' (Safe proxy bytecode)' if safe else '')+f'; snapshot amount {amt}')
            ex.append(t); safe and rv.append(t+('resolved: excluded per user decision t49s298',))
for a in SOLBURN:
    for comm,amt in rows.get(a,[]): ex.append((a,comm,'burn',SOLBURN[a]))
for a,v in sorted(c['sol'].items()):
    if v and v['owner']!=SYS:
        for comm,amt in rows[a]:
            t=(a,comm,'spl-token-account' if v['owner']=='TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' else 'program-owned-solana',f'getAccountInfo owner {v["owner"]}; snapshot amount {amt}')
            ex.append(t); rv.append(t+('resolved: excluded per user decision t49s298',))
hdr='address,community,reason,evidence\n'
open(R+'/eligibility/exclusions.csv','w').write(hdr+''.join(','.join(t)+'\n' for t in ex))
open(R+'/eligibility/review.csv','w').write(hdr.rstrip()+',status\n'+''.join(','.join(t)+'\n' for t in rv))
n7702=collections.Counter(comm for a,v in c['evm'].items() if v.startswith('0xef0100') for comm,_ in rows[a])
print('excl',len(ex),collections.Counter(t[2] for t in ex)); print('review',len(rv),collections.Counter(t[2] for t in rv))
print('7702 rows kept',sum(n7702.values()),dict(n7702))
print('sol no account (kept)',sum(1 for v in c['sol'].values() if v is None))
for t in ex: print(t)
