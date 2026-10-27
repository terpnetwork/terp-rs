# svg-gas-bench

Compares old (code 71, on-chain wasm) and new (code 98, `artifacts/cw721_svg.wasm`) cw721-svg in
`cosmwasm-vm` on the real wasm: instantiate, mint 1, and the `svg_token_uri` render query.

**These are not chain gas numbers.** "SDK est" = VM gas / 140_000 (wasmd v0.61.4
`DefaultGasMultiplier`) + KV writes at 2000 + 30/byte. Tx size, signature, ante handler, events and
the factory hop are not included.

```sh
./fetch-wasm.sh                      # code 71 from LCD, sha256-checked (gitignored)
cargo build --release
./target/release/svg-gas-bench akash bcna bitcoin bitsong cardano cosmos dao eth monero osmosis penumbra terp zec
```

Inputs: legacy `../svgs/interchain/<logo>/init.json` (code 71) and packed
`../svgs/interchain-packed/<logo>/init.json` (code 98). Outputs: `out/gas.json`,
`out/rendered/<logo>.code{71,98}.svg`. The render output is byte-identical between versions for all 13 logos.

Check the renders with `../sanitize` (DOMPurify + SMIL).

## Results (2026-10-27)

| logo | ver | init msg B | inst VM gas | inst SDK est | KV B | mint VM gas | mint SDK est | render VM gas |
|---|---|---|---|---|---|---|---|---|
| akash | 71 | 5382 | 401,445,376 | 396,197 | 12,511 | 299,967,378 | 36,062 | 159,091,945 |
| akash | 98 | 1833 | 3,261,846,946 | 118,188 | 2,563 | 1,747,097,215 | 50,859 | 2,385,496,587 |
| penumbra | 71 | 41026 | 1,713,707,940 | 2,900,490 | 95,675 | 857,637,746 | 42,235 | 1,066,842,295 |
| penumbra | 98 | 39405 | 36,332,876,825 | 1,199,810 | 30,743 | 1,804,757,797 | 53,461 | 34,715,791,290 |
| terp | 71 | 9816 | 740,803,188 | 709,121 | 22,861 | 630,109,881 | 68,930 | 256,829,375 |
| terp | 98 | 2955 | 4,679,669,423 | 153,546 | 3,404 | 3,880,029,315 | 96,604 | 3,972,335,056 |

All 13 logos: `out/gas.json`. Summary: instantiate is 2.4–4.6x cheaper (less storage). Mint is
about 1.3–1.4x more expensive (packed decode). Render VM gas is 15–33x higher. Penumbra is about
248k SDK-equivalent per query, under the default 3M smart-query limit.
