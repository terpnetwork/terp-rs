#!/bin/sh
# Local soundness gate for svg-mint-zrt.
# Proves the spend seed and note ledger survive a process restart, and that
# send cannot mark notes spent or broadcast. Does not touch ~/.terp or groot2.
set -eu
ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "unit tests"
cargo test -p svg-mint-zrt -- --test-threads=1

echo "build default and lab binaries"
cargo build -p svg-mint-zrt --features lab
DEF="$ROOT/target/debug/svg-mint-zrt"
LAB="$ROOT/target/debug/svg-mint-zrt"
# feature builds share the same target path; keep a copy of the lab binary
cp "$DEF" /tmp/svg-mint-zrt-lab.$$
# rebuild default after the lab copy so `unknown command` is the default binary
cargo build -p svg-mint-zrt
LAB="/tmp/svg-mint-zrt-lab.$$"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/svg-mint-zrt-e2e.XXXXXX")"
trap 'rm -rf "$WORK" "$LAB"' EXIT
export SVG_MINT_ZRT_DIR="$WORK/wallet"

echo "missing seed does not create a wallet"
if "$DEF" balance --json >"$WORK/out" 2>"$WORK/err"; then
  echo "balance succeeded without init"
  exit 1
fi
grep -q "refusing to create" "$WORK/err"
test ! -e "$SVG_MINT_ZRT_DIR/spend.seed"

echo "init then restart keeps the address"
"$DEF" init >"$WORK/addr1"
test -f "$SVG_MINT_ZRT_DIR/spend.seed"
ADDR1="$(cat "$WORK/addr1")"
case "$ADDR1" in
  u1*) ;;
  *) echo "address is not a unified address"; exit 1 ;;
esac
"$DEF" addresses >"$WORK/addr2"
grep -q "$ADDR1" "$WORK/addr2"
if "$DEF" init >"$WORK/addr3" 2>"$WORK/err"; then
  echo "second init must fail"
  exit 1
fi
grep -q "refusing" "$WORK/err"
ADDR_AGAIN="$(sed -n 's/^Address: //p' "$WORK/addr2")"
test "$ADDR_AGAIN" = "$ADDR1"

echo "default binary cannot fake a credit"
if "$DEF" lab-credit --nullifier aa --txid bb --zat 1 >"$WORK/out" 2>"$WORK/err"; then
  echo "default binary accepted lab-credit"
  exit 1
fi
grep -q "unknown command" "$WORK/err"

echo "lab credit survives restart; send does not spend"
NF="1111111111111111111111111111111111111111111111111111111111111111"
"$LAB" lab-credit --nullifier "$NF" --txid tx-e2e --zat 250000 --mined true
BAL1="$("$LAB" balance --json)"
printf '%s\n' "$BAL1" | grep -q '"total":250000'
# new process, same directory
BAL2="$("$DEF" balance --json)"
test "$BAL1" = "$BAL2"
if "$DEF" send --zat 1000 --address u1not-the-donation-ua >"$WORK/out" 2>"$WORK/err"; then
  echo "send to a foreign UA must fail"
  exit 1
fi
grep -q "donation" "$WORK/err"
DONATE="u1uml44spc4nn732t6605wjcwu3wn9dla4aehpyrvnhssq2ew3dk8mqk5e9jhnndzfny7fkrkx5ape05nzf8gwruqykkvc27fm0v9ln4hfrwkmtfrs27r06xu0uc9eneh777le3m8362v7xaurgtkgxtspy5eqhgpzw6axjwgwdy5wjdjt"
if "$DEF" send --zat 1000 --address "$DONATE" >"$WORK/out" 2>"$WORK/err"; then
  echo "send must not succeed without a prover"
  exit 1
fi
grep -q "prover is not linked" "$WORK/err"
grep -q "not marked spent" "$WORK/err"
BAL3="$("$DEF" balance --json)"
test "$BAL3" = "$BAL1"

echo "lab commit-spend is durable across another process"
"$LAB" lab-commit-spend --nullifier "$NF" --txid spend-e2e
BAL4="$("$DEF" balance --json)"
printf '%s\n' "$BAL4" | grep -q '"total":0'
"$DEF" notes | grep -q '"spent":true'

echo "e2e-zrt-wallet passed"
