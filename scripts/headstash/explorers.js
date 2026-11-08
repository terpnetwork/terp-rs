/**
 * Public-good tx indexers: find an *outgoing* signed tx for an address.
 * Blockscout / Etherscan-compatible `account.txlist` plus Blockscout v2.
 */
import { isRateLimited, sleep } from "./rpc-pool.js";

const DEFAULT_EXPLORERS = [
    { kind: "blockscout-v2", base: "https://eth.blockscout.com/api/v2" },
    { kind: "etherscan", base: "https://eth.blockscout.com/api" },
    { kind: "etherscan", base: "https://api.etherscan.io/api" },
];

function explorerList() {
    const raw = process.env.ETH_EXPLORER_URLS;
    if (raw && raw.trim()) {
        return raw.split(",").map((s) => {
            const u = s.trim();
            if (u.includes("/api/v2")) return { kind: "blockscout-v2", base: u.replace(/\/$/, "") };
            return { kind: "etherscan", base: u.replace(/\/$/, "") };
        });
    }
    return DEFAULT_EXPLORERS;
}

async function fetchJson(url, timeoutMs = 20_000) {
    const ac = new AbortController();
    const t = setTimeout(() => ac.abort(), timeoutMs);
    try {
        const res = await fetch(url, {
            signal: ac.signal,
            headers: { accept: "application/json" },
        });
        if (res.status === 429 || res.status === 503) {
            const err = new Error(`explorer HTTP ${res.status}`);
            err.status = res.status;
            throw err;
        }
        if (!res.ok) {
            const err = new Error(`explorer HTTP ${res.status}`);
            err.status = res.status;
            throw err;
        }
        return await res.json();
    } finally {
        clearTimeout(t);
    }
}

/** Latest outgoing tx hash, or null. */
export async function findOutgoingTxHash(address) {
    const explorers = explorerList();
    const key = process.env.ETHERSCAN_API_KEY || process.env.ETHERSCAN_KEY || "";
    let lastErr;
    for (const ex of explorers) {
        try {
            if (ex.kind === "blockscout-v2") {
                const url = `${ex.base}/addresses/${address}/transactions?filter=from`;
                const body = await fetchJson(url);
                const items = body?.items || body?.result || [];
                const tx = items.find((t) => (t.from?.hash || t.from || "").toLowerCase() === address.toLowerCase());
                const hash = tx?.hash || tx?.transaction_hash;
                if (hash) return hash;
                continue;
            }
            const qs = new URL(ex.base.includes("?") ? ex.base : ex.base);
            if (!ex.base.includes("?")) {
                qs.searchParams.set("module", "account");
                qs.searchParams.set("action", "txlist");
                qs.searchParams.set("address", address);
                qs.searchParams.set("startblock", "0");
                qs.searchParams.set("endblock", "99999999");
                qs.searchParams.set("page", "1");
                qs.searchParams.set("offset", "10");
                qs.searchParams.set("sort", "desc");
                if (key) qs.searchParams.set("apikey", key);
            }
            const body = await fetchJson(qs.toString());
            const rows = Array.isArray(body?.result) ? body.result : [];
            const out = rows.find(
                (t) =>
                    String(t.from || "").toLowerCase() === address.toLowerCase() &&
                    t.hash &&
                    t.isError !== "1"
            );
            if (out?.hash) return out.hash;
            if (typeof body?.result === "string" && /rate|limit/i.test(body.result)) {
                const err = new Error(body.result);
                err.status = 429;
                throw err;
            }
        } catch (err) {
            lastErr = err;
            if (isRateLimited(err) || err.status === 429) {
                console.warn(`[explorer] rate-limited ${ex.base} — rollover`);
                await sleep(800);
                continue;
            }
        }
    }
    if (lastErr && process.env.ETH_SCRAPE_DEBUG) {
        console.warn(`[explorer] no tx for ${address}: ${lastErr.message || lastErr}`);
    }
    return null;
}

export { DEFAULT_EXPLORERS };
