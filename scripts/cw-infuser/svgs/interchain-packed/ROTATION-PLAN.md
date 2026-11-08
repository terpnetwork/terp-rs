# Interchain Logos rotation: cw-orch dry-run plan (DRAFT, not executed)

Status: prepared only. No tx has been sent. The curator key does not exist yet.

## Parties
| role | address |
|---|---|
| factory | `terp1wg5z9rcwpw5kvrl4g78ees2l9ejptrhrx4t2k56nq6t27uly87zssujm80` |
| creator / curator | `LOGOS_CURATOR_TBD`: a dedicated logos curator key, **not created yet** |
| withdraw, royalties, wasm admin | Foundation DAO `terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w` |
| series | `interchain-logos` |

## Inputs
`svgs/interchain-packed/<logo>/init.json` are code-98 InstantiateMsgs produced by `pack_logos`.
`manifest.json` holds the sizes and packed sha256. Regenerate them once the curator key exists:

```sh
cargo run -p cw-infuser-scripts --bin pack_logos -- --creator <curator_addr> --withdraw terp14w2q...kw2w
```
`pack_logos` checks the variables, round-trips the packed data, enforces `MAX_SVG_SIZE` (42 KiB), and
runs `validate_animated_paint`, the same check the contract runs at instantiate. The dao template's
eye circles needed a static `fill` so the code-98 instantiate would accept it. Fixed in `dao/init.json`.

## Steps (cw-orch, Mock/dry-run first)
Use the suite helpers in `src/collection.rs`, following the Mock usage in `series_e2e.rs`.
1. **Mock rehearsal**:
   - `deploy_factory` on `Mock`, then `set_series("interchain-logos", cfg)`.
   - `instantiate_in_series` for all 13 logos from the packed init.json.
   - For each logo, mint 1 and check `svg_token_uri` against `gas-bench/out/rendered/<logo>.code98.svg`.
   - Run `sanitize/` over the renders.
2. **Daemon dry-run** (simulate only, no broadcast). Sender is the factory owner:
   `SetSeries{series:"interchain-logos", config}` with the DAO as admin and withdraw address.
3. Sender is the curator. For each logo:
   `CreateSeriesCollection{series:"interchain-logos", instantiate_msg, label:"interchain-logos-<logo>"}`.
   Use `CreateSvgCollection{instantiate_msg,label}` if a series is not wanted.
4. Post-check: each collection's `contract_info.admin` is the DAO, and `withdraw_address` and royalties are the DAO.
5. Optional: retire the old code-71 collections (0 tokens each). Doing so needs admin
   `terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052`.

## Tx size and batching (2 MiB tx limit)
- The largest message is penumbra at 39,405 B. All 13 together are about 85 KB, so size is never the constraint.
- Gas is the constraint. Estimated SDK gas for instantiate, excluding the factory hop and fixed costs:
  penumbra about 1.20M, osmosis 0.38M, cardano 0.27M, the rest 0.11–0.19M each. The total is about 3.3M.
- Suggested batches:
  - **A**: penumbra alone, about 1.3M or more after overhead.
  - **B**: osmosis, cardano, bcna, cosmos, terp, bitcoin.
  - **C**: akash, bitsong, dao, eth, monero, zec.
- Set the gas limit to about 1.5x the simulated value, and check the block max_gas.
