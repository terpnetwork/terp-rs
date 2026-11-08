import { readCsvFile, loadProjectAddresses } from "./utils.js";
import { SigningKey, Transaction } from "ethers";
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { HEADSTASH_YAML } from "./constants.js";
import { readYamlFile } from "./utils.js";
import {
    RpcPool,
    resolveRpcMode,
    rpcUrlList,
    sleep,
} from "./rpc-pool.js";
import { findOutgoingTxHash } from "./explorers.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DATA_DIR = path.join(__dirname, "scripts-data");
const MAP_CSV = path.join(DATA_DIR, "address_pubkey_map.csv");
const DEAD_CSV = path.join(DATA_DIR, "dead_addresses.csv");
const PROGRESS_JSON = path.join(DATA_DIR, "pubkey_scrape_progress.json");

function yamlPath() {
    const fromEnv = process.env.HEADSTASH_YAML;
    if (fromEnv) return fromEnv;
    const candidates = [
        path.join(__dirname, HEADSTASH_YAML),
        path.join(__dirname, "../../tools/headstash/circuit/headstash.yaml"),
        path.join(__dirname, "../../tools/headstash/headstash.yaml"),
    ];
    return candidates.find((p) => fs.existsSync(p)) || candidates[1];
}

function resolveCsv(yamlFile, csvRel) {
    if (path.isAbsolute(csvRel)) return csvRel;
    const fromYaml = path.resolve(path.dirname(yamlFile), csvRel);
    if (fs.existsSync(fromYaml)) return fromYaml;
    const fromScripts = path.resolve(__dirname, csvRel);
    if (fs.existsSync(fromScripts)) return fromScripts;
    return fromYaml;
}

function ensureDataDir() {
    fs.mkdirSync(DATA_DIR, { recursive: true });
    if (!fs.existsSync(MAP_CSV)) {
        fs.writeFileSync(MAP_CSV, "addr,pubkey_compressed,pubkey_uncompressed,tx\n");
    }
    if (!fs.existsSync(DEAD_CSV)) {
        fs.writeFileSync(DEAD_CSV, "addr,reason\n");
    }
}

function normAddr(s) {
    return String(s || "").trim().replace(/^0x/i, "").toLowerCase();
}

function loadHavePubkey(file) {
    const set = new Set();
    if (!fs.existsSync(file)) return set;
    for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/).slice(1)) {
        if (!line.trim()) continue;
        const [addr, pk] = line.split(",");
        if (pk && pk.trim().startsWith("0x") && pk.trim().length >= 66) {
            set.add(normAddr(addr));
        }
    }
    return set;
}

function loadPermanentDead(file) {
    const set = new Set();
    if (process.env.ETH_RETRY_DEAD === "all") return set;
    if (!fs.existsSync(file)) return set;
    const keep = new Set(["contract", "never-sent", "recover-mismatch"]);
    for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/).slice(1)) {
        if (!line.trim()) continue;
        const cols = line.split(",");
        const reason = (cols[1] || "").trim();
        if (keep.has(reason)) set.add(normAddr(cols[0]));
    }
    return set;
}

function appendRow(file, cols) {
    const line =
        cols.map((c) => String(c ?? "").replace(/,/g, "")).join(",") + "\n";
    fs.appendFileSync(file, line);
}

/** Recover compressed (33-byte) + uncompressed (65-byte) secp256k1 pubkeys from a signed tx. */
export function recoverPubkeyFromTx(tx) {
    if (!tx?.signature) return null;
    try {
        const rebuilt = Transaction.from({
            type: tx.type,
            to: tx.to,
            from: tx.from,
            nonce: tx.nonce,
            gasLimit: tx.gasLimit,
            gasPrice: tx.gasPrice,
            maxFeePerGas: tx.maxFeePerGas,
            maxPriorityFeePerGas: tx.maxPriorityFeePerGas,
            data: tx.data,
            value: tx.value,
            chainId: tx.chainId,
            signature: tx.signature,
            accessList: tx.accessList,
        });
        const uncompressed = SigningKey.recoverPublicKey(
            rebuilt.unsignedHash,
            rebuilt.signature
        );
        const compressed = SigningKey.computePublicKey(uncompressed, true);
        return { compressed, uncompressed };
    } catch {
        return null;
    }
}

/**
 * Single address. `provider` may be ethers Provider *or* RpcPool.
 * Kept for the self-hosted path (`provider.getHistory` if present).
 */
export async function processAddress(address, provider) {
    const pool =
        provider instanceof RpcPool
            ? provider
            : {
                  async getTransaction(h) {
                      return provider.getTransaction(h);
                  },
                  async getTransactionCount(a) {
                      return provider.getTransactionCount(a);
                  },
                  async otsGetTxBySenderNonce(a, n) {
                      if (typeof provider.send !== "function") return null;
                      return provider.send("ots_getTransactionBySenderAndNonce", [
                          a,
                          n,
                      ]);
                  },
                  async withProvider(fn) {
                      return fn(provider);
                  },
              };

    // 1) Public explorer (no JSON-RPC). Then fetch the signed tx via the RPC pool.
    const explorerHash = await findOutgoingTxHash(address);
    if (explorerHash) {
        const tx = await pool.getTransaction(explorerHash);
        const keys = recoverPubkeyFromTx(tx);
        if (keys) return { address, ...keys, tx: explorerHash };
    }

    // 2) ethers v5-style history (self-hosted indexer / custom provider)
    if (typeof provider.getHistory === "function") {
        try {
            const history = await provider.getHistory(address);
            if (history?.length) {
                const tx = history[history.length - 1];
                const keys = recoverPubkeyFromTx(tx);
                if (keys) return { address, ...keys, tx: tx.hash };
            }
        } catch {
            /* fall through */
        }
    }

    // 3) Otterscan (self-hosted Erigon / geth+ots) — skip on public CF-gated RPCs
    try {
        const nonce = await pool.getTransactionCount(address);
        if (nonce > 0) {
            const raw = await pool.otsGetTxBySenderNonce(address, nonce - 1);
            const hash = raw?.hash || raw;
            if (hash && typeof hash === "string") {
                const tx = await pool.getTransaction(hash);
                const keys = recoverPubkeyFromTx(tx);
                if (keys) return { address, ...keys, tx: tx.hash };
            }
        }
    } catch {
        /* no ots — expected on public RPC */
    }

    return null;
}

async function mapLimit(items, limit, fn) {
    const ret = new Array(items.length);
    let i = 0;
    async function worker() {
        while (i < items.length) {
            const idx = i++;
            ret[idx] = await fn(items[idx], idx);
        }
    }
    const n = Math.max(1, Math.min(limit, items.length || 1));
    await Promise.all(Array.from({ length: n }, worker));
    return ret;
}

function collectHolders(yamlFile, projects) {
    const list = Array.isArray(projects) ? projects : Object.values(projects || {});
    const holders = [];
    const seen = new Set();
    for (const proj of list) {
        if ((proj.chain_type || "evm") !== "evm") continue;
        const csvPath = resolveCsv(yamlFile, proj.csv);
        if (!fs.existsSync(csvPath)) {
            console.warn(`[pubkeys] missing csv ${csvPath} (${proj.name})`);
            continue;
        }
        holders.push({ proj: proj.name, csvPath });
    }
    return holders;
}

/**
 * Original entry: one provider, all yaml projects.
 */
export async function determineAllPubkeys(provider) {
    const yp = yamlPath();
    const distributionData = await readYamlFile(yp);
    const pool =
        provider instanceof RpcPool
            ? provider
            : new RpcPool(rpcUrlList(resolveRpcMode()));
    // If they passed a lone provider, wrap via a 1-url pool when possible
    return scrapeEligiblePubkeys({
        pool: provider instanceof RpcPool ? pool : provider,
        yamlFile: yp,
        projects: distributionData.projects,
    });
}

export async function scrapeEligiblePubkeys({
    pool,
    yamlFile,
    projects,
    concurrency,
    limit,
} = {}) {
    ensureDataDir();
    const yp = yamlFile || yamlPath();
    let data = { projects: projects || [] };
    if (!projects) {
        try {
            data = await readYamlFile(yp);
        } catch (err) {
            console.warn(`[pubkeys] yaml not loaded (${yp}): ${err.message}`);
            data = { projects: [] };
        }
    }
    const conc = Number(concurrency || process.env.ETH_SCRAPE_CONCURRENCY || 3);
    const cap = Number(limit || process.env.ETH_SCRAPE_LIMIT || 0);

    const havePk = loadHavePubkey(MAP_CSV);
    const permDead = loadPermanentDead(DEAD_CSV);
    const skip = new Set([...havePk, ...permDead]);

    const extraCsv = process.env.ETH_ADDR_CSV;
    const sources = collectHolders(yp, data.projects);
    if (extraCsv) sources.push({ proj: "ETH_ADDR_CSV", csvPath: extraCsv });
    let addresses = [];
    const extraAddr = process.env.ETH_ADDR;
    if (extraAddr && /^0x[0-9a-fA-F]{40}$/.test(extraAddr.trim())) {
        addresses.push(extraAddr.trim());
    }
    for (const src of sources) {
        const records = await loadProjectAddresses(src.csvPath);
        for (const r of records) {
            const addr = String(r.addr || r.address || "").trim();
            if (!/^0x[0-9a-fA-F]{40}$/.test(addr)) continue;
            const key = normAddr(addr);
            if (skip.has(key)) continue;
            skip.add(key);
            addresses.push(addr);
        }
    }
    if (cap > 0) addresses = addresses.slice(0, cap);

    console.log(
        `[pubkeys] yaml=${yp} query=${addresses.length} skip_pubkey=${havePk.size} skip_permanent_dead=${permDead.size} concurrency=${conc}`
    );

    let ok = 0;
    let stale = 0;
    let fail = 0;
    await mapLimit(addresses, conc, async (address) => {
        try {
            const result = await processAddress(address, pool);
            if (result?.compressed) {
                appendRow(MAP_CSV, [
                    result.address,
                    result.compressed,
                    result.uncompressed,
                    result.tx || "",
                ]);
                ok++;
            } else {
                appendRow(DEAD_CSV, [address, "no-signed-tx"]);
                stale++;
            }
        } catch (err) {
            fail++;
            console.warn(`[pubkeys] ${address}: ${err.message || err}`);
            await sleep(400);
        }
        const n = ok + stale + fail;
        if (n % 25 === 0) {
            fs.writeFileSync(
                PROGRESS_JSON,
                JSON.stringify(
                    {
                        ok,
                        stale,
                        fail,
                        remainingHint: addresses.length - n,
                        at: new Date().toISOString(),
                    },
                    null,
                    2
                )
            );
            console.log(`[pubkeys] progress ok=${ok} stale=${stale} fail=${fail}`);
        }
    });

    const summary = { ok, stale, fail, map: MAP_CSV, dead: DEAD_CSV };
    fs.writeFileSync(PROGRESS_JSON, JSON.stringify({ ...summary, at: new Date().toISOString() }, null, 2));
    console.log("✅ Finished.", summary);
    return summary;
}

export async function runPubkeyScrapeCli() {
    const mode = resolveRpcMode();
    const urls = rpcUrlList(mode);
    console.log(`[pubkeys] RPC mode=${mode} endpoints=${urls.length} first=${urls[0]}`);
    const pool = new RpcPool(urls);
    return scrapeEligiblePubkeys({ pool });
}

export {
    yamlPath,
    MAP_CSV,
    DEAD_CSV,
    DATA_DIR,
};
