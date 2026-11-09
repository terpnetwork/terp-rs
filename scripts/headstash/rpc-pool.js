/**
 * JSON-RPC pool: local geth and/or public-good Ethereum endpoints.
 * Rate-limit (429 / -32005 / "too many requests") → backoff, then next URL.
 */
import { JsonRpcProvider, FetchRequest } from "ethers";

const DEFAULT_PUBLIC_RPCS = [
    "https://public.1rpc.io/eth",
    "https://eth-mainnet.g.alchemy.com/public",
    "https://0xrpc.io/eth",
    "https://cloudflare-eth.com",
];

const LOCAL_RPC = process.env.GETH_HTTP_URL || "http://127.0.0.1:8545";

function parseUrlList(envVal, fallback) {
    if (!envVal || !String(envVal).trim()) return fallback;
    return String(envVal)
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
}

function isRateLimited(err) {
    const msg = String(err?.info?.responseBody || err?.shortMessage || err?.message || err || "").toLowerCase();
    const code = err?.code ?? err?.error?.code ?? err?.info?.error?.code;
    const status = err?.info?.responseStatus ?? err?.status;
    if (status === 429 || status === 503 || status === 403 || status === 401) return true;
    if (code === -32005 || code === 429) return true;
    return (
        msg.includes("rate limit") ||
        msg.includes("too many requests") ||
        msg.includes("over rate") ||
        msg.includes("call rate") ||
        msg.includes("exceeded") ||
        msg.includes("capacity") ||
        msg.includes("just a moment") ||
        msg.includes("cloudflare") ||
        msg.includes("403 forbidden")
    );
}

function sleep(ms) {
    return new Promise((r) => setTimeout(r, ms));
}

/**
 * @param {"local"|"public"|"auto"} mode
 */
export function rpcUrlList(mode) {
    const extra = parseUrlList(process.env.ETH_RPC_URLS, []);
    const publicList = extra.length ? extra : DEFAULT_PUBLIC_RPCS;
    const local = process.env.ETH_RPC_URL || LOCAL_RPC;
    if (mode === "local") return [local];
    if (mode === "public") return publicList;
    // auto: local first, then public goods
    const rest = publicList.filter((u) => u !== local);
    return [local, ...rest];
}

export function resolveRpcMode() {
    const m = (process.env.ETH_RPC_MODE || "auto").toLowerCase();
    if (m === "local" || m === "geth" || m === "self-hosted") return "local";
    if (m === "public" || m === "pg" || m === "public-goods") return "public";
    return "auto";
}

export class RpcPool {
    /**
     * @param {string[]} urls
     */
    constructor(urls, { maxBackoffMs = 30_000 } = {}) {
        this.urls = urls.filter(Boolean);
        if (!this.urls.length) throw new Error("RpcPool: no RPC URLs");
        this.maxBackoffMs = maxBackoffMs;
        this.i = 0;
        this.disabledUntil = new Array(this.urls.length).fill(0);
        this.providers = this.urls.map((url) => {
            const req = new FetchRequest(url);
            req.timeout = 25_000;
            return new JsonRpcProvider(req, 1, { staticNetwork: true });
        });
    }

    currentUrl() {
        return this.urls[this.i];
    }

    _pick() {
        const now = Date.now();
        for (let n = 0; n < this.urls.length; n++) {
            const idx = (this.i + n) % this.urls.length;
            if (this.disabledUntil[idx] <= now) {
                this.i = idx;
                return idx;
            }
        }
        let soonest = 0;
        for (let n = 1; n < this.urls.length; n++) {
            if (this.disabledUntil[n] < this.disabledUntil[soonest]) soonest = n;
        }
        this.i = soonest;
        return soonest;
    }

    _backoff(idx, attempt) {
        const wait = Math.min(this.maxBackoffMs, 400 * 2 ** Math.min(attempt, 8));
        this.disabledUntil[idx] = Date.now() + wait;
        return wait;
    }

    /**
     * Run `fn(provider)` with rollover across the pool.
     */
    async withProvider(fn, { attempts = 12 } = {}) {
        let lastErr;
        for (let attempt = 0; attempt < attempts; attempt++) {
            const idx = this._pick();
            const provider = this.providers[idx];
            try {
                return await fn(provider, this.urls[idx]);
            } catch (err) {
                lastErr = err;
                if (isRateLimited(err)) {
                    const wait = this._backoff(idx, attempt);
                    console.warn(
                        `[rpc] rate-limited ${this.urls[idx]} — wait ${wait}ms, rollover`
                    );
                    await sleep(wait);
                    this.i = (idx + 1) % this.urls.length;
                    continue;
                }
                // transient network
                const msg = String(err?.message || err);
                if (
                    /timeout|econnreset|enotfound|socket|502|503|504/i.test(msg)
                ) {
                    this._backoff(idx, attempt);
                    this.i = (idx + 1) % this.urls.length;
                    await sleep(200 * (attempt + 1));
                    continue;
                }
                throw err;
            }
        }
        throw lastErr;
    }

    async getTransaction(hash) {
        return this.withProvider((p) => p.getTransaction(hash));
    }

    async getTransactionCount(address) {
        return this.withProvider((p) => p.getTransactionCount(address));
    }

    /** Otterscan / Erigon (self-hosted indexer). */
    async otsGetTxBySenderNonce(address, nonce) {
        return this.withProvider((p) =>
            p.send("ots_getTransactionBySenderAndNonce", [address, nonce])
        );
    }
}

export { DEFAULT_PUBLIC_RPCS, LOCAL_RPC, isRateLimited, sleep };
