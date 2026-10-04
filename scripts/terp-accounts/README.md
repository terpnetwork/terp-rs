# Terp NFT Scripts

## Contents

This library contains both Rust & Bash scripts for the BS-NFT repository.

| Name | Language | Version | Description |
|----------|----------|----------|----------|
| [**Testing Suite**](./src/test/mod.rs) | `Rust`   | `tbd`  | Integration test suite for all contracts.  |
| [**Cw-Orch Deployment**](./src/deploy/mod.rs)  | `Rust`   | `tbd`   | Used for production and simulation environment contract deployment workflows.  |
| **Terp Account Framework Deployment**  | `Rust` |`tbd`  | Automation scripts for deployment of smart contract and IBC infrastructure that powers Terp Accounts.   |

## Current Orchestrator Suites

| Suite Name | Description |
|----------|----------|
| [`TerpAccountSuite`](./src/deploy/bundles/account.rs#12)| Account Collection, Marketplace, and Minter. |

## Cw-Orchestrator Commands

| Command | Description |
|----------|----------|
| `cargo test` | Run all test in codebase |
| `cargo run --bin manual_deploy -- --network [<testnet>,<mainnet>,<local>] --method <load_from,deploy_on>` | Deploy workflow for all contracts needed for terp-accounts. |

## Bash Commands

before running, `sh/.env.testnet` to `sh/.env`.

| Command | Description |
|----------|----------|
| `./sh/1_upload` | store wasm code |
| `./sh/2a_init_marketplace.sh` | instantiate marketplace, make sure you have updated `.env` with code-ids. |
| `./sh/2b_init_minter.sh` | instantiate minter. make sure you have updated `.env` with Marketplace address (MKT). |
| `./sh/3_setup_minter` | setup marketplace . Update .env with both the minter and collection addresses (MINTER and COLLECTION).|
<!-- | `sh broadcast.sh` | Broadcast a transaction or a message to the network or chain |
| `sh exec_accept_bid.sh` | accept bid as account owner|
| `sh exec_add_text.sh` | add text records to an account token |
| `sh exec_assoc.sh` | associate a smart contract or address with a given account token |
| `sh exec_bid.sh` | bid on an account token |
| `sh exec_mint.sh` | mint a new account token via account inter|
| `sh exec_mint_specific_user.sh` | mint an account token to a specific user |
| `sh exec_minter_update_config.sh` | update account minter config |
| `sh exec_pause.sh` | Pause contract execution, temporarily halting specific functions or operations |
| `sh exec_update_public_time.sh` | update public time mint start |
| `sh exec_update_verifier.sh` | update verifier oracle contract |
| `sh query_ask.sh` | query asks on a given account token|
| `sh query_asks_by_renew_time.sh` | Query asks sorted by renewal time, |
| `sh query_bids_sorted_by_price.sh` | Query bids sorted by price,  |
| `sh query_col.sh` | Query the collection, |
| `sh query_lookup.sh` | Query the account name |
| `sh query_metadata.sh` | Query metadata, retrieving additional information or attributes associated with a specific account |
| `sh query_minter.sh` | Query the minter, retrieving information about the account minter |
| `sh query_mkt.sh` | Query the market, retrieving information about available assets, prices, and market conditions |
| `sh query_mkt_bids_by_seller.sh` | Query market bids by seller, displaying bids placed by a specific seller or entity |
| `sh query_mkt_params.sh` | Query market parameters, retrieving configuration settings and rules governing the market |
| `sh query_token_info.sh` | Query token information, retrieving details and attributes associated with a specific token |
| `sh query_tokens.sh` | Query tokens, retrieving a list of available tokens on the chain or in a specific collection |
| `sh query_tx.sh` | Query transaction information, retrieving details about a specific transaction or set of transactions | -->