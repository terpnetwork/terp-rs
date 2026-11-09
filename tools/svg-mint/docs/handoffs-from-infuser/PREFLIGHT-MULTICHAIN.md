# Preflight: can the runtimes see payments to ephemeral dests?

This is the check before FE treats checkout as live. It does **not** send money. It confirms each rail watcher can actually talk to a node, and that HashMerchant is the temp fee-granter — not the runtime.

**Self-hosted porch + house:** `PORCH-MULTICHAIN-HANDOFF.md`. On groot2, `SVG_MINT_REQUIRE_SELF_HOSTED_RPC` defaults **on**: Alchemy/Infura/publicnode URLs fail even if chainId looks right. Loopback, `192.168.10.0/24`, and lab Docker names (`anvil`, `bitcoind`) pass. Extra hosts: `DREGG_RPC_ALLOWLIST`.

Run **on groot2** (where `svg-mint-fed` and HashMerchant live):

```bash
bash crates/dregg/svg-mint/scripts/preflight-multichain.sh
```

Or from a grok session that can SSH there:

```
/workflow dregg-multichain-preflight
```

The script never prints seed phrases, HD seeds, or RPC passwords.

## What “sound” means

DREGG issues a **fresh** ETH/Base `0x`, BTC `bc1`, and ZIP-321 dest per checkout (`GET /v1/pay-targets`). Watchers must see inbound payments to **those** addresses:

| Rail | Env the fed process must have | Probe |
|---|---|---|
| Ethereum | `ETH_RPC` | `eth_chainId` = `0x1`; pay-targets `eth_addr` |
| Base | `BASE_RPC` | `eth_chainId` = `0x2105`; `base_addr` same 0x family |
| Bitcoin | `BTC_RPC` | `getblockchaininfo`; pay-targets `btc_addr` |
| Zcash | `ZCASH_DEVTOOL_BIN` (Linux ELF) + `ZCASH_DEVTOOL_SERVER` (`127.0.0.1:9067`) | pay-targets `zec_uri` non-null, or hide ZEC |
| Native Terp | `DREGG_MINTER_LCD` + `DREGG_MINTER_ADDR` | LCD balances; no `DREGG_MINTER_FUNDED` lie |
| HD identity | persisted `DREGG_HD_SEED` | dests survive process restart |
| HashMerchant | `ONBOARD_RUNTIME` ≠ `ONBOARD_HASHMERCHANT` | LCD authz grants runtime→HM; HM has uthiol |

ZEC may be skipped only with `PREFLIGHT_ALLOW_ZEC_SKIP=1` (FE must keep the ZEC button hidden). That is not full multichain sound.

## After this is green

1. Runtime → HashMerchant authz: `AUTHZ_BROADCAST=1 cargo run --example hashmerchant_authz_bootstrap -p ict-rs-cw-orch`
2. Fund the HashMerchant account so it can pay grant fees and the 1 THIOL runway
3. Restart HashMerchant / DREGG so they inherit the RPC env
