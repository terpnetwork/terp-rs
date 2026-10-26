#!/usr/bin/env bash
# Rebuild the Headstash eligibility root with the budget approved by
# BASE_ALLOCATION = 67,000,000 TERP split by headstash.yaml allocation_percentage.
# See README.md in this directory. Read-only w.r.t. any network.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"            # tools/headstash
TOTAL_UTERP="${TOTAL_UTERP:-67000000000000}"  # BASE_ALLOCATION 67,000,000 TERP (of TOTAL_SUPPLY 420,000,000)
OUT="${OUT:-$ROOT/target/eligibility}"
STAGE="$ROOT/target/eligibility-stage"
# Per-community budgets: floor(TOTAL * allocation_percentage) using the exact
# decimal strings from headstash.yaml; remainder handed out 1 uterp at a time by
# largest fractional part (ties: project name asc). Sum == TOTAL exactly.
BUDGETS="$(python3 - "$ROOT/headstash.yaml" "$TOTAL_UTERP" <<'P'
import re,sys
from decimal import Decimal, getcontext
getcontext().prec=80
t=open(sys.argv[1]).read(); total=int(sys.argv[2])
names=re.findall(r'- name: (\S+)',t); pc=re.findall(r'allocation_percentage: ([0-9.eE-]+)',t)
assert len(names)==len(pc)
s=sum(Decimal(p) for p in pc)
rows=[]
for n,p in zip(names,pc):
    exact=Decimal(total)*Decimal(p)/s      # normalise so shares sum to 1 exactly
    fl=int(exact); rows.append([n,fl,exact-fl])
rem=total-sum(r[1] for r in rows)
for r in sorted(rows,key=lambda r:(-r[2],r[0]))[:rem]: r[1]+=1
assert sum(r[1] for r in rows)==total
for n,b,_ in rows: print(f"{n}={b}")
P
)"
rm -rf "$STAGE"; mkdir -p "$STAGE"
ARGS=(--decimals chronic-token=18)
EXCLUDE="${EXCLUDE:-$HERE/exclusions.csv}"   # burn/zero/contract rows: no leaf, share stays unallocated
[ -n "$EXCLUDE" ] && [ "$EXCLUDE" != none ] && ARGS+=(--exclude "$EXCLUDE")
while IFS= read -r kv; do
  p="${kv%%=*}"; ln -s "$ROOT/headstash/communities/$p" "$STAGE/$p"; ARGS+=(--budget "$kv")
done <<< "$BUDGETS"
echo "$BUDGETS" > "$OUT.budgets.txt" 2>/dev/null || true
cd "$ROOT"
cargo run --release -q -p headstash-epoch --bin headstash-eligibility -- "$STAGE" "$OUT" "${ARGS[@]}"
