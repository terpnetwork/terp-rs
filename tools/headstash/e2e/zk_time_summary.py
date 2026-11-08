import json,sys,re,statistics as S
d=json.load(open(sys.argv[1])); fb=d['fb_samples']
def fin(h):
    v=[x[3] for x in fb if x[1]==h]; return v[0]*1000 if len(v)==1 else None
agg={}
for r in d['runs']:
    a=agg.setdefault(r['name'],{'gas':[],'exec':[],'fin':[],'zk':[],'ver':[],'load':[]})
    a['gas'].append(r['gas_used']); e=r['wasm_metric_delta'].get('wasm_contract_execute_sum')
    if e is not None and r['wasm_metric_delta'].get('wasm_contract_execute_count')==1: a['exec'].append(e)
    f=fin(r['height']); 
    if f is not None: a['fin'].append(f)
    for l in r['zk']:
        m=dict(re.findall(r'(\w+)=(\S+)',l))
        if 'total_us' in m: a['zk'].append(int(m['total_us'])/1000); a['ver'].append(int(m['verify_us'])/1000); a['load'].append(int(m.get('load_us',m.get('load_prep_us')))/1000)
med=lambda x: round(S.median(x),2) if x else None
print("name gas exec_ms(n) finalize_ms(n) host_total_ms host_verify_ms host_loadprep_ms")
for k,a in agg.items():
    print(k, med(a['gas']), med(a['exec']),len(a['exec']), med(a['fin']),len(a['fin']), med(a['zk']), med(a['ver']), med(a['load']), "verify_range", (round(min(a['ver']),1),round(max(a['ver']),1)) if a['ver'] else '')
print(d['uptime'])
