#!/usr/bin/env node
// Size benchmark of the on-chain claim page bundle (fix4).
// Uses e2e/render_frontend_prop.py bundle() itself (via python3), so the gzip bytes and 128 KiB chunking are exactly
// what chunk-upload stores on chain; brotli (q11) is measured here with node zlib for comparison only (not uploaded).
//   node bench-size.mjs [--prover-dir DIR] [--page-dir DIR] [--out size-report.json] [--diff previous-report.json] [--json]
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { brotliCompressSync, constants as Z } from "node:zlib";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
const HERE = dirname(fileURLToPath(import.meta.url));
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d; };
const pageDir = arg("--page-dir", HERE), proverDir = arg("--prover-dir", process.env.HS_PROVER_DIR || "/tmp/hsfe/prover");
const out = arg("--out", join(HERE, "size-report.json")), prev = arg("--diff", existsSync(out) ? out : null);
const RENDER = join(HERE, "..", "e2e");
const py = `
import sys, json, hashlib; sys.path.insert(0, sys.argv[1]); import render_frontend_prop as r
m, chunks, fs = r.bundle(sys.argv[2], sys.argv[3])
print(json.dumps({"chunk": r.CHUNK, "files": [dict(f, dir=d, chunk_sizes=[len(c) for c in cs]) for (d, _), f, cs in zip(r.PAGE_FILES, fs, chunks)]}))`;
const b = JSON.parse(execFileSync("python3", ["-c", py, RENDER, pageDir, proverDir], { encoding: "utf8" }));
const files = b.files.map((f) => {
  const raw = readFileSync(join(f.dir === "page" ? pageDir : proverDir, f.path));
  const br = brotliCompressSync(raw, { params: { [Z.BROTLI_PARAM_QUALITY]: 11, [Z.BROTLI_PARAM_SIZE_HINT]: raw.length } }).length;
  return { path: f.path, raw: f.raw_size, gzip: f.gzip_size, brotli: br, chunks: f.chunks, chunk_sizes: f.chunk_sizes, sha256: f.sha256 };
});
const sum = (k) => files.reduce((s, f) => s + f[k], 0);
const rep = { v: 1, chunk_bytes: b.chunk, encoding_uploaded: "gzip -9 (mtime 0)", files,
  total: { files: files.length, raw: sum("raw"), gzip: sum("gzip"), brotli: sum("brotli"), chunks: sum("chunks") } };
let old = null; if (prev && existsSync(prev)) try { old = JSON.parse(readFileSync(prev, "utf8")); } catch { old = null; }
const kb = (n) => (n / 1024).toFixed(1).padStart(8), d = (a, b) => (b == null ? "" : ` (${a - b >= 0 ? "+" : ""}${a - b})`);
const of = (p) => old && old.files.find((x) => x.path === p);
if (process.argv.includes("--json")) console.log(JSON.stringify(rep, null, 2));
else {
  console.log("file                        raw KiB  gzip KiB  br KiB  chunks");
  for (const f of files) console.log(`${f.path.padEnd(26)}${kb(f.raw)}${kb(f.gzip)}${kb(f.brotli)}  ${String(f.chunks).padStart(4)}${d(f.gzip, of(f.path)?.gzip)}`);
  const t = rep.total; console.log(`${"TOTAL".padEnd(26)}${kb(t.raw)}${kb(t.gzip)}${kb(t.brotli)}  ${String(t.chunks).padStart(4)}${old ? d(t.gzip, old.total.gzip) + " gzip bytes vs previous, chunks" + d(t.chunks, old.total.chunks) : ""}`);
}
writeFileSync(out, JSON.stringify(rep, null, 2) + "\n");
