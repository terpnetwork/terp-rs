# Scripts

These are helping scripts for un
This script checks for and sums any duplicate allocation together to generate a csv file that does not contain duplicate addresses.

| Command   | Description | Files Created |
|-----------|-------------|---------------|
| `cargo run -p headstash-eth-pubkeys --release` / `just scrape-eth-pubkeys` | **Rust** parallel explorers + **batched** `eth_getTransactionByHash` | `scripts-data/address_pubkey_map.csv` |
| `node scrape-pubkeys.js` / `just scrape-eth-pubkeys-js` | Same job in JS (kept) | same |
| `node main.js -12` | JS scrape (`ETH_RPC_MODE=local` keeps a single `ETH_RPC_URL` / geth) | same |
| `node main.js -1` | Runs full workflow to generate accurate genesis distribution | `scripts-data/final-output.csv`, `points-distribution.csv`, `total-points.csv` |
| `node main.js -2` | Prepares accurate allocations from genesis snapshots (`gaia.csv`, `bcna_delegators.csv`) | `scripts-data/final-output.csv`, `points-distribution.csv`, `total-points.csv` |
| `node main.js -3` | Analyzes exported state (`morocco-1`) to split accounts into active/inactive | `accounts-active.json`, `accounts-inactive.json` |
| `node main.js -4` | Compares expected vs. actual genesis allocations, flags discrepancies | Log output, discrepancy reports |
| `node main.js -5` | Summarizes differences between old and new distribution | Summary files in `scripts-data/` |

## ETH pubkey scrape (inclusion set)

Self-hosted geth is unchanged (`scripts/geth/build.yml`). Public-good RPCs are the default when geth is down.

| Env | Meaning |
|-----|---------|
| `ETH_RPC_MODE` | `auto` (local then public), `local`, `public` |
| `ETH_RPC_URL` | Single URL (legacy / local) |
| `ETH_RPC_URLS` | Comma-separated RPC list (overrides the public defaults) |
| `ETH_EXPLORER_URLS` | Comma-separated Blockscout/Etherscan bases |
| `ETHERSCAN_API_KEY` | Optional, for api.etherscan.io volume |
| `ETH_SCRAPE_CONCURRENCY` | Default 3 |
| `ETH_SCRAPE_LIMIT` | Cap addresses this run (resume skips done rows) |
| `ETH_ADDR` / `ETH_ADDR_CSV` | Extra address or csv (`addr` column) |

Outputs: `scripts/scripts-data/address_pubkey_map.csv` (compressed + uncompressed secp256k1) for note generation.

Reruns **skip** any address already in the map (has a pubkey) and permanent dead (`contract`, `never-sent`, `recover-mismatch`). Transient misses (`no-signed-tx`, `recover-failed`) are queried again. `ETH_RETRY_DEAD=all` retries every dead row too.

After a scrape (or `just enrich-eth-pubkeys` / `--enrich-only`), each community snapshot is copied to `<name>.enriched.csv` with `pubkey_compressed`, `pubkey_uncompressed`, `tx`, `pubkey_status`. Original `addr,amount` CSVs are never overwritten.

Poseidon-v1 notes (rayon): `just gen-community-notes` → `artifacts/community-notes/{notes.json,merkle_output.json}`. Only `pubkey_status=ok` rows; burn/unallocated skipped. Leaves use recovered compressed secp256k1 pubkeys.

Holders **without** a recoverable pubkey are listed in `<name>.unallocated.csv` (`addr,amount,class,reason`) and rolled up in `headstash/communities/UNALLOCATED.md`. Those snapshot amounts are the **burn** set, not minted allocations. Classes: `dead` (contract / never-sent / recover-mismatch) vs `blind` (pending / no-signed-tx / recover-failed).

## Adding A New Community For Headstash (EVM based)

### Step 1: Distribution Snapshot

- Date snapshot was taken
- .csv file with `addr,amount` as headers
- create new folder in `../headstash/communities/<new-community>`

### Step 2: Percentile Distribution Calculation

- calculate desired points for percentile range & desired token per point

### Step 3: Addition To Scripts

```js
    {
        csv: "../headstash/communities/<new-community>/<distribution>.csv", 
        points: [
            { points: 1, min: 1, max: 1 },   // 1st - nth percentile
            { points: 2, min: 2, max: 9 }, // n+1 - m percentile
            { points: 3, min: 10, max: 10 } // m+1 - 100th percentile
        ], 
        tpp: 1660.079051 // calculated token per point
    },
```

## Adding A New Community For Headstash (Solana based)
