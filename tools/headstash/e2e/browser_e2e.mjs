// Playwright driver for browser_e2e.py. Reads JSON from stdin; prints one JSON line.
import { createRequire } from "module"; const require = createRequire(process.env.NODE_PATH + "/");
const { chromium } = require("playwright");
const phase = process.argv[2];
const inp = JSON.parse(await new Promise((r) => { let s = ""; process.stdin.on("data", (d) => (s += d)); process.stdin.on("end", () => r(s)); }));
const out = { phase, ok: false };
const browser = await chromium.launch({ headless: true });
const page = await (await browser.newContext({ acceptDownloads: true })).newPage();
out.net = []; page.on("requestfailed", (r) => out.net.length < 20 && console.error(r.method() + " " + r.url().slice(0, 120) + " " + (r.failure() || {}).errorText)); page.on("console", (m) => m.type() === "error" && console.error("console " + m.text().slice(0, 200)));
let last = null; page.on("download", (d) => (last = d));
// CSP has no unsafe-eval: wait via locators, never waitForFunction/evaluate
const logText = () => page.locator("#log").textContent().catch(() => "");
const saveLast = async (dst) => { const t0 = Date.now(); while (!last && Date.now() - t0 < 30000) await page.waitForTimeout(200); if (last) { await last.saveAs(dst); out.saved = true; last = null; } };
const counts = async () => JSON.parse((await page.locator("#summary").getAttribute("data-counts")) || "{}");
try {
  const t0 = Date.now(); await page.goto(inp.url);
  await page.locator("body[data-ready='1'], body[data-error]").waitFor({ timeout: 180000 });
  const err = await page.locator("body").getAttribute("data-error"); if (err) throw new Error("loader: " + err);
  out.loaded = true; out.load_ms = Date.now() - t0; out.version = await page.locator("#fe-ver").textContent();
  if (phase === "register") {
    await page.fill("#key", inp.key_hex); await page.setInputFiles("#elig", inp.elig); await page.fill("#reward", inp.reward);
    await page.click("#gen"); await page.locator("#log", { hasText: /generated \d+ notes|ERROR/ }).waitFor({ timeout: 600000 });
    out.key_field_cleared = (await page.inputValue("#key")) === ""; last = null;
    await page.click("#register");
    await page.locator("#log", { hasText: /notes file saved[\s\S]*notes file saved|ERROR|failed/ }).waitFor({ timeout: 900000 });
    await saveLast(inp.out); out.counts = await counts();
  } else {
    await page.setInputFiles("#nf", inp.notes); await page.click("#load");
    await page.locator("#log", { hasText: /loaded \d+ notes|ERROR/ }).waitFor({ timeout: 60000 });
    last = null; const t1 = Date.now(); await page.click("#claim");
    await page.locator("#log", { hasText: /notes file saved|no accepted anchor/ }).waitFor({ timeout: 3600000 });
    out.claim_ms = Date.now() - t1; await saveLast(inp.out); out.counts = await counts();
    await page.fill("#pen", "penumbra1xxxx"); await page.click("#pen-check"); out.penumbra_invalid_rejected = /invalid/.test(await page.locator("#pen-out").textContent());
    await page.click("#how"); out.modal_visible = await page.locator("#modal").isVisible();
  }
  out.ok = true;
} catch (e) { out.error = String(e.message || e).slice(0, 400); }
out.log = await logText(); await browser.close(); console.log(JSON.stringify(out));
