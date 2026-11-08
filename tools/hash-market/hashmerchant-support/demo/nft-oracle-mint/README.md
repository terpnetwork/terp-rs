# Demo: Oracle-priced NFT mint + loyalty whitelist + feegrant

**Hard rule:** HashMerchant / oracle data supplies **price mid or bounds only**.  
It must **never** mint NFT supply or fungible value by itself.

## Pieces

| Piece | Tool |
|-------|------|
| Eligible members | `hm-support whitelist build` |
| Gas sponsorship | `hm-support accounts feegrant` |
| Price | Sidecar / on-chain root or price attestation (`price/oracle` scope) |
| Mint | CW721 + minter (or program minter) gated by whitelist proof |

## Offline prep (no chain)

```bash
cd tools/hashmerchant-support
./hm-support.sh whitelist self-test
./hm-support.sh whitelist build \
  --in whitelist/fixtures/members.json \
  --out /tmp/loyalty-wl
./hm-support.sh accounts feegrant \
  --file whitelist/fixtures/members.json \
  --granter ops \
  --dry-run
```

## Lab / testnet recipe

1. **Path B smoke** — sidecar healthy; optional root confirmed  
   `HASHMERCHANT_SIDECAR_URL=… ./hm-support.sh smoke`
2. **Bootstrap members** — real keyring addresses  
   `./hm-support.sh accounts bootstrap --file members.json --out members.resolved.json --feegrant --granter ops`
3. **Whitelist root**  
   `./hm-support.sh whitelist build --in members.resolved.json --out tree/`
4. **Deploy minter + CW721** — pin code ids; set admin to ops  
   (chain-specific; see `recipe.sh` for CLI sketch)
5. **Price sync** — read oracle mid → set `unit_price` or payment bound  
   Prefer ±bps band + max supply. Reject stale heights.
6. **Mint** — eligible user, fee-granter ops, merkle proof in msg:

```bash
# Real msg surface: contracts/cw721/hm-nft-minter (hm-loyalty/v1 proof object)
# proofs/$ADDR.json from: hm-support whitelist build --out tree/
PROOF_JSON=$(jq -c '{codec,address,leaf,proof,root}' "tree/proofs/${ADDR}.json")
# Quote first:
# terpd query wasm contract-state smart "$MINTER" '{"mint_price_bound":{}}'
MID=$(terpd query wasm contract-state smart "$MINTER" '{"mint_price_bound":{}}' -o json \
  | jq -r '.data.mid // .data.min')
terpd tx wasm execute "$MINTER" \
  "{\"mint\":{\"proof\":${PROOF_JSON}}}" \
  --amount "${MID}uterp" \
  --from "$USER_KEY" \
  --fee-granter "$GRANTER_ADDR" \
  --fees 400000uterp -y
```

Admin Mode C price sync (demo fallback when sudo not wired):

```bash
terpd tx wasm execute "$MINTER" \
  '{"admin_sync_price":{"mid":"1500000","foreign_height":1,"asset_id":"ZEC"}}' \
  --from ops -y
```

Set whitelist root:

```bash
ROOT=$(cat tree/root.txt)
terpd tx wasm execute "$MINTER" \
  "{\"set_whitelist_root\":{\"root\":\"${ROOT}\",\"codec\":\"hm-loyalty/v1\",\"enabled\":true}}" \
  --from ops -y
```

See also: `crates/terp-rs/contracts/infuser/` and generated clients under `crates/terp-rs/api/ts/hm-nft-minter`.

7. **Negative checks**
   - Non-member proof → reject  
   - No feegrant / empty granter → user pays or fails  
   - Stale oracle (if enforced) → reject underpayment

## Website alignment

`websites/TODO.md` already plans: query eligibility → put merkle path in mint msg.  
Serve `tree/proofs/<address>.json` from a static API for demos.

## Security

- Feegrant: spend limit + allowed msgs + expiry  
- Leaves: address only (hashed with domain `hm-loyalty/v1|`) — no emails  
- Mainnet: `HM_MAINNET_OK=true` required on scripts  
- Oracle never increases `max_tokens` or mints balances
