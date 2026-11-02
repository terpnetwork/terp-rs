// Times the sound raw-key claim prover (prove_rawkey_fixture_browser: params+keygen in client, prove, verify)
// under node, single-threaded wasm. Usage: node bench_wasm_rawkey.mjs <pkg dir> <k> <destination>
import { readFileSync, copyFileSync, mkdtempSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
const [dir, k, dest] = [process.argv[2], +process.argv[3], process.argv[4]];
const tmp = mkdtempSync(join(tmpdir(), "hsbench-rk-"));
copyFileSync(join(dir, "zk_headstash.js"), join(tmp, "zk_headstash.mjs"));
const mod = await import(join(tmp, "zk_headstash.mjs"));
const phases = []; let last = performance.now();
const orig = console.error;
console.error = (m) => { const t = performance.now(); phases.push({ phase: String(m).split(" ")[0], secs: +((t - last) / 1000).toFixed(3), info: String(m) }); last = t; };
mod.initSync({ module: readFileSync(join(dir, "zk_headstash_bg.wasm")) });
const t0 = performance.now(); last = t0;
let out;
try { out = JSON.parse(mod.prove_rawkey_fixture_browser(k, dest)); } catch (e) { console.error = orig; console.log(JSON.stringify({ error: String(e), phases })); process.exit(1); }
const t1 = performance.now();
console.error = orig;
console.log(JSON.stringify({ k, total_secs: +((t1 - t0) / 1000).toFixed(3), ...out, phases, runtime: "node " + process.version }));
