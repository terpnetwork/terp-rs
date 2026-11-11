// k19 lab task 3: threaded wasm raw-key prover in headless Chromium (COOP/COEP, 127.0.0.1 only).
// HS_REG=1 benches the two-step registration (step 1) circuit instead.
// Usage: node bench_mt.mjs <pkg dir> <k> <threads,...>   (PW = path to playwright module dir)
import http from "node:http";
import { readFileSync, existsSync, statSync } from "node:fs";
import { join, extname } from "node:path";
import { createRequire } from "node:module";
const [dir, k, tl] = [process.argv[2], +process.argv[3], process.argv[4] || "1,4,8"];
const { chromium } = createRequire(import.meta.url)(process.env.PW);
const page_html = `<!doctype html><script type="module">
import init, * as m from "./pkg/zk_headstash.js";
const REG = ${process.env.HS_REG ? "true" : "false"};
window.run = async (threads, k) => {
  const log = []; let last = performance.now();
  console.error = (s) => { const t = performance.now(); log.push([String(s).split(" ")[0], +((t-last)/1000).toFixed(3), String(s)]); last = t; };
  await init();
  if (threads > 0 && m.initThreadPool) await m.initThreadPool(threads);
  if (REG) m.rawkey_set_reg_mode(true);
  const T = () => performance.now(); const r = { threads: m.rawkey_threads(), cross_origin_isolated: crossOriginIsolated };
  let t = T(); const art = m.rawkey_keygen_artifacts(k); r.keygen_regen_secs = (T()-t)/1000; r.artifact_bytes = art.length;
  t = T();
  const db = await new Promise((ok, no) => { const q = indexedDB.open("hs", 1); q.onupgradeneeded = () => q.result.createObjectStore("a"); q.onsuccess = () => ok(q.result); q.onerror = no; });
  await new Promise((ok, no) => { const x = db.transaction("a", "readwrite"); x.objectStore("a").put(art, "k"+k); x.oncomplete = ok; x.onerror = no; });
  r.idb_store_secs = (T()-t)/1000;
  t = T();
  const back = await new Promise((ok, no) => { const q = db.transaction("a").objectStore("a").get("k"+k); q.onsuccess = () => ok(q.result); q.onerror = no; });
  r.idb_read_secs = (T()-t)/1000;
  last = T(); t = T(); r.out = JSON.parse(m.prove_rawkey_cached(back, "terp1lab")); r.cached_total_secs = (T()-t)/1000;
  r.phases = log; return r;
};
</script>`;
const types = { ".js": "text/javascript", ".wasm": "application/wasm", ".html": "text/html" };
const srv = http.createServer((q, s) => {
  let p = q.url.split("?")[0]; let f = p === "/" ? null : p.startsWith("/pkg/") ? join(dir, p.slice(5)) : null;
  if (f && !f.startsWith(dir)) f = null;
  s.setHeader("Cross-Origin-Opener-Policy", "same-origin"); s.setHeader("Cross-Origin-Embedder-Policy", "require-corp");
  if (p === "/") { s.setHeader("content-type", "text/html"); return s.end(page_html); }
  if (f && existsSync(f) && statSync(f).isDirectory()) f = join(f, "zk_headstash.js");
  if (!f || !existsSync(f)) { s.statusCode = 404; return s.end(); }
  s.setHeader("content-type", types[extname(f)] || "application/octet-stream"); s.end(readFileSync(f));
}).listen(0, "127.0.0.1");
await new Promise((r) => srv.on("listening", r));
const url = `http://127.0.0.1:${srv.address().port}/`;
const browser = await chromium.launch({ headless: true });
for (const n of tl.split(",").map(Number)) {
  const ctx = await browser.newContext(); const page = await ctx.newPage();
  page.on("pageerror", (e) => console.log("pageerror", String(e)));
  await page.goto(url); await page.waitForFunction(() => window.run);
  try { const r = await page.evaluate(([n, k]) => window.run(n, k), [n, k]); console.log(JSON.stringify({ k, req_threads: n, ...r })); }
  catch (e) { console.log(JSON.stringify({ k, req_threads: n, error: String(e) })); }
  await ctx.close();
}
await browser.close(); srv.close();
