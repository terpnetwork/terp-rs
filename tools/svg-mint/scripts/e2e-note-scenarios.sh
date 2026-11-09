#!/bin/sh
# Soundness scenarios for checkout note identity.
# A mempool note is receipt, not a mint. Match is the checkout address or memo, never "one shielded tx".
set -eu
ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
out="$(cargo test -p dregg --lib zec_notes:: -- --test-threads=1 2>&1)" || {
  printf '%s\n' "$out"
  echo "zec_notes scenarios failed"
  exit 1
}
printf '%s\n' "$out"
for name in \
  empty_memo_mempool_matches_only_this_address \
  mined_note_is_the_block_signal \
  second_checkout_does_not_take_the_first_address \
  unmined_match_is_receipt_not_a_mint \
  orchard_output_is_not_accepted \
  ironwood_receiver_inside_ua_matches
do
  printf '%s\n' "$out" | grep -q "$name" || {
    echo "missing test $name"
    exit 1
  }
done
echo "zec_notes scenarios passed"
echo "wallet restart suite"
sh "$ROOT/scripts/e2e-zrt-wallet.sh"
