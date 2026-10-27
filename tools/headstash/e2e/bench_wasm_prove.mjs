// Times the dapp's browser prover (zk_headstash wasm, prove_note_witness_browser) under node.
// Witness JSON on stdin (never echoed); prints {secs, proof_bytes}. Single-threaded wasm, as in the browser worker.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { copyFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
const dir = process.argv[2];
const tmp = mkdtempSync(join(tmpdir(), "hsbench-wasm-"));
copyFileSync(join(dir, "zk_headstash.js"), join(tmp, "zk_headstash.mjs"));  // ESM glue outside a type:module package
const mod = await import(join(tmp, "zk_headstash.mjs"));
const wit = readFileSync(0, "utf8");
const t0 = performance.now();
mod.initSync({ module: readFileSync(join(dir, "zk_headstash_bg.wasm")) });
const t1 = performance.now();
const out = JSON.parse(mod.prove_note_witness_browser(wit));
const t2 = performance.now();
console.log(JSON.stringify({ init_secs: +((t1 - t0) / 1000).toFixed(3), prove_secs: +((t2 - t1) / 1000).toFixed(3),
  proof_bytes: out.proof_bytes ?? (out.proof_hex ? out.proof_hex.length / 2 : null), runtime: "node " + process.version }));
