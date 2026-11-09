# SVG mint MVP lock (deploy + FE handoff)

Refunds and IBC-only mint are **out**. Goal: DREGG runtime sees ZEC/ETH/BTC deposits, then **cw-orch `Daemon.execute` hopalong mint** on live `cw721_svg`. FE gets a frozen component list. Live `cw-svg-minter` code 72 **does not export `migrate`** — do not migrate it.

## Live contracts (cw-orch state)

From groot `~/.cw-orchestrator/state.json` **morocco-1** (mainnet):

| Contract | code_id | address |
|---|---|---|
| `cw-svg-minter` | 72 | `terp188jpppxtfpf0vvt5gypdj52hk2xuswwcn38auczzxv20fcxz3kss9fhlru` |
| `cw721_svg` wasm | **83** | minter `update_code_id` (new collections) |
| `cw721_svg` live | 71 | `terp1vp7smemrv846hpsyqrzlnqapmu3sv7hyygf4rrnw3ls26ezk864qupr5zt` |

`state_local.json` `120u-1` `cw-svg-minter` `terp1wug8sewp6cedgkmrmvhl3lf3tulagm9hnvy8p0rppz9yjw0g4wtqw39a47` is the **local ict-rs lab** address (same as `svg_mint_local_prod`). Do **not** migrate that as 120u-1.

## Work remaining (this MVP)

1. **DREGG runtime (ETH+BTC+ZEC):** watch vault logs / BTC txs / ZIP-321 ZEC memos. Rails carry **Pedersen `memo_hex`**, never `terp1`. Entitle the HTTPS recipient.
2. **Mint on morocco-1 collection** (`svg_mint_cw_orch_runtime` → `COLLECTION` = `cw721_svg`). Runtime key pays 100 TERP|THIOL. `runtime_minters` is cw-gate-only; code 72 cannot be migrated.
3. **Rails:** ETH **or** BTC **or** ZEC (ZIP-321). Refunds out.
4. **FE:** `FE-HANDOFF.md` — three-rail checkout + memo = Terp wallet.

## Out

Refunds, ICS-08 handshake, mint-only-from-packet, HashMerchant, Groth16 settlement.

## Runtime identity

DREGG mint: `~/.terp/dregg-svg-runtime.mnemonic` (derived terp1). Collection/minter admin: `~/.terp/svg-minter-admin.mnemonic` (`terp1fxcc…`). Never the same key.
