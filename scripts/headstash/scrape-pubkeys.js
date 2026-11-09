#!/usr/bin/env node
/**
 * Accumulate eligible EVM holder pubkeys for Headstash notes.
 *
 *   ETH_RPC_MODE=auto|local|public node scrape-pubkeys.js
 *   ETH_SCRAPE_LIMIT=20 node scrape-pubkeys.js
 *   ETH_ADDR_CSV=./one.csv node scrape-pubkeys.js
 *
 * Local geth: docker compose -f geth/build.yml up  (still supported).
 * Public: 1rpc / alchemy public / 0xrpc / cloudflare-eth — rollover on 429.
 */
import { runPubkeyScrapeCli } from "./pubkeys.js";

runPubkeyScrapeCli().catch((err) => {
    console.error(err);
    process.exit(1);
});
