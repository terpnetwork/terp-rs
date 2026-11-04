// Playwright driver for browser_e2e_v2.py (claim page v2). Reads JSON from stdin; prints one JSON line.
// EVM side: an injected TEST EIP-1193 provider (window.ethereum; signs in Node with the page's own crypto.js, EIP-191).
// Penumbra side: the REAL Zafu extension (inp.zafu_ext = unpacked release dir) in a persistent profile copied from an
// onboarded throwaway wallet (inp.zafu_profile, password inp.zafu_pass). Harness-only steps: storage prefs
// {approvalSurface:"popup", Penumbra network enabled+active}, unlocking via popup.html, and clicking "connect" in the approval popup.
import { createRequire } from "module"; import { pathToFileURL } from "url"; const require = createRequire(process.env.NODE_PATH + "/");
const { chromium } = require("playwright");
const phase = process.argv[2];
const inp = JSON.parse(await new Promise((r) => { let s = ""; process.stdin.on("data", (d) => (s += d)); process.stdin.on("end", () => r(s)); }));
const out = { phase, ok: false };
const fs = await import("fs");
let browser = null, ctx, zx = null;
if (inp.zafu_ext) {
  const prof = `${inp.zafu_profile}-run-${phase}-${process.pid}`; fs.cpSync(inp.zafu_profile, prof, { recursive: true });
  ctx = await chromium.launchPersistentContext(prof, { channel: "chromium", headless: true, acceptDownloads: true, viewport: { width: 1100, height: 900 },
    args: [`--disable-extensions-except=${inp.zafu_ext}`, `--load-extension=${inp.zafu_ext}`] });
  const sw = ctx.serviceWorkers()[0] || (await ctx.waitForEvent("serviceworker")); const extId = sw.url().split("/")[2];
  await sw.evaluate(async () => chrome.storage.local.set({ approvalSurface: "popup", enabledNetworks: ["zcash", "penumbra"], activeNetwork: "penumbra" }));
  const lp = await ctx.newPage(); await lp.goto(`chrome-extension://${extId}/popup.html`); await lp.waitForTimeout(2500);
  if (await lp.locator("input[type=password]").count()) { await lp.locator("input[type=password]").first().fill(inp.zafu_pass); await lp.keyboard.press("Enter"); await lp.waitForTimeout(3000); }
  zx = { extId, approvals: 0 };
  ctx.on("page", async (p) => { try { await p.waitForLoadState(); if (!p.url().includes("#/approval/origin")) return; await p.waitForTimeout(1200);
    await p.getByText("connect", { exact: true }).click(); zx.approvals++; } catch (e) { console.error("approval click: " + e.message); } });
  await lp.close();
} else { browser = await chromium.launch({ headless: true }); ctx = await browser.newContext({ acceptDownloads: true, viewport: { width: 1100, height: 900 } }); }
const page = await ctx.newPage();
page.on("console", (m) => m.type() === "error" && console.error("console " + m.text().slice(0, 200)));
let last = null; page.on("download", (d) => (last = d));
const reqs = []; page.on("request", (r) => reqs.push({ t: Date.now(), url: r.url(), body: r.postData() || "" }));
const shot = async (n) => { await page.screenshot({ path: `${inp.shots}/${phase}-${n}.png`, fullPage: true }); (out.shots ||= []).push(`${inp.shots}/${phase}-${n}.png`); };
const text = (s) => page.locator(s).textContent().catch(() => "");
const attr = (s, a) => page.locator(s).getAttribute(a).catch(() => null);
const until = async (fn, ms) => { const t0 = Date.now(); while (Date.now() - t0 < ms) { const v = await fn(); if (v) return v; await page.waitForTimeout(250); } return fn(); };
const saveLast = async (dst) => { await until(async () => !!last, 30000); if (last) { await last.saveAs(dst); out.saved = true; last = null; } };
const logCount = async (re) => ((await text("#log")).match(re) || []).length;

// test EIP-1193 provider
let C = null, priv = null; out.seed_signs = 0;
if (inp.key_hex) {
  C = await import(pathToFileURL(inp.crypto).href);
  priv = C.unhex(inp.key_hex); delete inp.key_hex;
  await page.exposeFunction("__hsEth", async (method, params) => {
    if (method === "eth_requestAccounts" || method === "eth_accounts") return [inp.evm_addr];
    if (method === "eth_chainId") return "0x1";
    if (method === "personal_sign") {
      const m = C.unhex(params[0]); if (new TextDecoder().decode(m).startsWith("terp-headstash-note-seed-v2")) out.seed_signs++;
      const s = await C.sign(priv, C.keccak256(C.cat(C.te.encode("\x19Ethereum Signed Message:\n" + m.length), m)));
      return "0x" + C.hex(C.cat(s.r, s.s, new Uint8Array([27 + (s.rec & 1)])));
    }
    throw new Error("test provider: unsupported " + method);
  });
  await page.addInitScript(() => { window.ethereum = { isMetaMask: true, request: ({ method, params }) => window.__hsEth(method, params || []) }; });
}
const pen = { list: [] };

try {
  const t0 = Date.now(); await page.goto(inp.url);
  await page.locator("body[data-ready='1'], body[data-error]").waitFor({ timeout: 180000 });
  const err = await attr("body", "data-error"); if (err) throw new Error("loader: " + err);
  out.loaded = true; out.load_ms = Date.now() - t0; out.version = await text("#fe-ver");
  await page.locator("#hal-status[data-state='ok'], #hal-status[data-state='error']").waitFor({ timeout: 120000 });
  if (phase === "v2a") {
    out.code_ids = await page.inputValue("#code-ids"); await page.click("#browse");
    await until(async () => (await attr("#inst-list", "data-count")) !== null || (await text("#inst-err")), 30000);
    out.instances = Number(await attr("#inst-list", "data-count")); out.selected = (await text("#hs-cur")).trim();
    out.hal_state = await attr("#hal-status", "data-state"); out.hal_sha = await attr("#hal-status", "data-sha"); out.hal_text = await text("#hal-status");
    await shot("01-instance-hal");
    // tampered local copy must be refused, then restore from the pinned source
    await page.setInputFiles("#hal-file", inp.tampered);
    await page.locator("#hal-status[data-state='error']").waitFor({ timeout: 30000 }).catch(() => {});
    out.hal_tamper_state = await attr("#hal-status", "data-state"); out.hal_tamper_err = (await text("#hal-err")) || (await text("#hal-status"));
    await shot("02-hal-tampered-refused");
    await page.click("#hal-fetch"); await page.locator("#hal-status[data-state='ok']").waitFor({ timeout: 60000 }).catch(() => {});
    out.hal_restored = await attr("#hal-status", "data-state");
    // info modal on every step
    const btns = page.locator("button.info[data-info]"); out.modals = await btns.count(); out.modals_ok = 0;
    for (let i = 0; i < out.modals; i++) {
      const b = btns.nth(i), id = await b.getAttribute("data-info"); await b.click();
      const vis = await page.locator("#info-dlg").isVisible(), st = await attr("#info-dlg", "data-step"), ti = (await text("#info-title")).trim(), wr = [(await text("#info-what")), (await text("#info-reveals")), (await text("#info-risks"))].every((x) => x.trim().length > 10);
      if (i === 2) await shot("03-info-modal-eligibility");
      await page.click("#info-close"); const closed = !(await page.locator("#info-dlg").isVisible());
      if (vis && st === id && ti && wr && closed) out.modals_ok++;
    }
    // eligibility: address from the EIP-1193 provider, lookup local; no request may carry the address
    const tA = Date.now(); await page.click("#mm-connect");
    await until(async () => (await attr("#elig-out", "data-notes")) !== null || (await text("#elig-err")), 30000);
    const id = inp.evm_addr.toLowerCase().replace(/^0x/, ""); const win = reqs.filter((r) => r.t >= tA);
    out.lookup_requests = win.length; out.leak = win.filter((r) => (r.url + r.body).toLowerCase().includes(id)).map((r) => r.url.slice(0, 100));
    out.elig_notes = Number(await attr("#elig-out", "data-notes")); out.elig_text = await text("#elig-out");
    out.reward_mode = await page.locator("input[name=reward-mode][value=ephemeral]").isChecked();
    await shot("04-eligible");
    last = null; await page.click("#derive");
    await until(async () => (await attr("#summary", "data-counts")) || (await text("#reg-err")), 600000);
    if (await text("#reg-err")) throw new Error("derive: " + (await text("#reg-err")));
    await saveLast(inp.out_derive); out.reward_addr = await text("#reward-addr");
    const n0 = await logCount(/notes file saved/g); last = null; await page.click("#register");
    await until(async () => (await logCount(/notes file saved/g)) > n0 || (await text("#reg-err")), 900000);
    if (await text("#reg-err")) throw new Error("register: " + (await text("#reg-err")));
    await saveLast(inp.out); out.counts = JSON.parse((await attr("#summary", "data-counts")) || "{}");
    await shot("05-registered");
  } else {
    await page.setInputFiles("#nf", inp.notes); await page.click("#load");
    await until(async () => /loaded \d+ notes|ERROR/.test(await text("#log")) || (await text("#reg-err")), 60000);
    await page.click("#anchor-check"); await page.waitForTimeout(1500); out.anchor = await text("#anchor-out");
    await shot("06-loaded-anchor");
    const runClaim = async (name) => {
      const n0 = await logCount(/notes file saved/g), e0 = await text("#claim-err"); last = null; const t1 = Date.now();
      await page.click("#claim");
      await until(async () => (await logCount(/notes file saved/g)) > n0 || ((await text("#claim-err")) && (await text("#claim-err")) !== e0), 1800000);
      out[name + "_ms"] = Date.now() - t1; out[name + "_err"] = await text("#claim-err"); out[name + "_out"] = await text("#claim-out");
      await shot(name);
    };
    await page.fill("#claim-max", "1");
    await runClaim("07-claim-ephemeral");
    await page.check("input[name=dest-mode][value=forward]"); await page.waitForTimeout(300);
    out.channels = await page.locator("#fwd-channel option").allTextContents();
    out.zafu_detect = await text("#zafu-detect");
    if (!zx) throw new Error("forward-zafu needs the real Zafu extension (zafu_ext)");
    await page.click("#zafu-connect");
    await page.locator("#zafu-detect[data-state='connected'], #zafu-detect[data-state='error']").waitFor({ timeout: 90000 });
    out.zafu_state = await attr("#zafu-detect", "data-state"); out.zafu_text = await text("#zafu-detect"); out.zafu_approvals = zx.approvals;
    await shot("08a-zafu-connected");
    await runClaim("08-forward-zafu"); out.fwd_preview = await text("#fwd-preview");
    const z1 = await page.inputValue("#recv"); pen.list.push(z1);
    for (let i = 0; i < 2; i++) { await page.click("#zafu"); await until(async () => (await page.inputValue("#recv")) !== pen.list[pen.list.length - 1] || (await text("#dst-err")), 30000); pen.list.push(await page.inputValue("#recv")); }
    out.zafu_book = await page.evaluate(() => JSON.parse(localStorage.getItem("terp-hs-zafu-indices-v1") || "[]"));
    out.zafu_addrs_distinct = new Set(pen.list).size === pen.list.length && pen.list.every((x) => /^penumbra1[02-9ac-hj-np-z]{134}$/.test(x));
    out.zafu_addr_prefixes = pen.list.map((x) => x.slice(0, 18));
    await page.click("#zafu-disconnect"); await page.waitForFunction(() => document.querySelector("#zafu-detect")?.getAttribute("data-state") !== "connected", null, { timeout: 10000 }).catch(() => {}); out.zafu_after_disconnect = await attr("#zafu-detect", "data-state");
    await page.check("#recv-any"); await page.fill("#recv", inp.recv_b);
    await runClaim("09-forward-chain-b");
    await page.fill("#recv", z1); // the Zafu address already used by claim 08: must be refused by the reuse guard
    await runClaim("10-reuse-refused"); out.reuse_err = out["10-reuse-refused_err"];
    await page.click("#save"); await saveLast(inp.out);
    out.counts = JSON.parse((await attr("#summary", "data-counts")) || "{}");
  }
  out.ok = true;
} catch (e) { out.error = String(e.message || e).slice(0, 400); await shot("error").catch(() => {}); }
out.penumbra_addrs = pen.list.length; out.real_zafu = !!zx; out.log = await text("#log"); await (browser ? browser.close() : ctx.close()); console.log(JSON.stringify(out));
