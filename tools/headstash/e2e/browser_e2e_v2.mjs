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
let last = null, dls = []; page.on("download", (d) => { last = d; dls.push(d.suggestedFilename()); });
const reqs = []; page.on("request", (r) => reqs.push({ t: Date.now(), url: r.url(), body: r.postData() || "" }));
const shot = async (n) => { await page.screenshot({ path: `${inp.shots}/${phase}-${n}.png`, fullPage: true }); (out.shots ||= []).push(`${inp.shots}/${phase}-${n}.png`); };
const text = (s) => page.locator(s).textContent().catch(() => "");
const attr = (s, a) => page.locator(s).getAttribute(a).catch(() => null);
const until = async (fn, ms) => { const t0 = Date.now(); while (Date.now() - t0 < ms) { const v = await fn(); if (v) return v; await page.waitForTimeout(250); } return fn(); };
const saveLast = async (dst) => { await until(async () => !!last, 30000); if (last) { await last.saveAs(dst); out.saved = true; last = null; } };
const logCount = async (re) => ((await text("#log")).match(re) || []).length;
// R13 helpers: viewport shots (sheet), section status snapshot, expand a collapsed component, open a headstash's sheet
const vshot = async (n) => { await page.screenshot({ path: `${inp.shots}/${phase}-${n}.png` }); (out.shots ||= []).push(`${inp.shots}/${phase}-${n}.png`); };
const SECS = ["eligibility", "register", "save", "wallet", "claim"];
const sections = () => page.evaluate((ss) => Object.fromEntries(ss.map((x) => [x, document.getElementById("sec-" + x).dataset.status + (document.getElementById("sec-" + x + "-body").hidden ? "" : "+open")])), SECS);
const expand = async (x) => { if (await page.locator(`#sec-${x}-body`).isHidden()) await page.click(`#sec-${x}-toggle`); await page.locator(`#sec-${x}-body`).waitFor({ state: "visible", timeout: 5000 }); };
const sheetOpen = () => page.evaluate(() => document.getElementById("hs-sheet").open);
const openHs = async (addr) => { await until(async () => (await page.locator(`#inst-list button[data-addr="${addr}"]`).count()) > 0, 30000);
  await page.click(`#inst-list button[data-addr="${addr}"]`); await until(sheetOpen, 30000);
  await page.locator("#hal-status[data-state='ok'], #hal-status[data-state='error']").waitFor({ timeout: 120000 }); };
const CLAIM_UI = ["#claim", "#mm-connect", "#derive", "#register", "#save", "#zafu-connect", "#privacy", "#hal-fetch", "#nf", "#claim-pick"];
const landingHides = async () => { const vis = []; for (const q of CLAIM_UI) if (await page.locator(q).isVisible()) vis.push(q); return { sheet_open: await sheetOpen(), visible: vis }; };

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

// R12 tamper: serve a malicious headstash theme from the LCD (base64 of {"theme":{}}); the page must reject it
if (phase === "theme" && inp.fake_wallets) await page.addInitScript((os) => { const s = Symbol.for("penumbra"); const g = (window[s] ||= {});
  for (const o of os) g[o] = { manifest: o + "/manifest.json", connect: async () => { throw new Error("fake wallet: test only"); }, isConnected: () => false, request: async () => {}, disconnect: async () => {} }; }, inp.fake_wallets);
if (phase === "theme" && inp.tamper) await page.route("**/smart/eyJ0aGVtZSI6e319*", (r) => r.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ data: inp.tamper }) }));
try {
  const t0 = Date.now(); await page.goto(inp.url);
  await page.locator("body[data-ready='1'], body[data-error]").waitFor({ timeout: 180000 });
  const err = await attr("body", "data-error"); if (err) throw new Error("loader: " + err);
  out.loaded = true; out.load_ms = Date.now() - t0; out.version = await text("#fe-ver");
  if (phase !== "theme") { out.landing = await landingHides(); out.hs_cur_before = (await text("#hs-cur")).trim(); }
  if (phase === "v2a") {
    out.code_ids = await page.inputValue("#code-ids"); await page.click("#browse");
    await until(async () => (await attr("#inst-list", "data-count")) !== null || (await text("#inst-err")), 30000);
    out.instances = Number(await attr("#inst-list", "data-count"));
    await shot("00-landing");
    // info modal on the landing view (the sheet makes the landing inert once open)
    const infoOk = async (b) => { const id = await b.getAttribute("data-info"); await b.click();
      const vis = await page.locator("#info-dlg").isVisible(), st = await attr("#info-dlg", "data-step"), ti = (await text("#info-title")).trim(), wr = [(await text("#info-what")), (await text("#info-reveals")), (await text("#info-risks"))].every((x) => x.trim().length > 10);
      await page.click("#info-close"); const closed = !(await page.locator("#info-dlg").isVisible()); return vis && st === id && ti && wr && closed; };
    const lb = page.locator("#landing button.info[data-info]"); out.modals = await lb.count(); out.modals_ok = 0;
    for (let i = 0; i < out.modals; i++) if (await infoOk(lb.nth(i))) out.modals_ok++;
    // R13 sheet: open, aria-modal, focus inside + contained, Esc / close button, focus back to the opener, reduced motion
    const sh = {}; await openHs(inp.contract); await page.waitForTimeout(600); out.selected = (await text("#hs-cur")).trim();
    Object.assign(sh, await page.evaluate(() => { const d = document.getElementById("hs-sheet"), r = d.getBoundingClientRect();
      return { open: d.open, aria_modal: d.getAttribute("aria-modal"), focus_in: d.contains(document.activeElement), focus_id: document.activeElement?.id, anim: getComputedStyle(d).animationName, bottom: Math.round(innerHeight - r.bottom), landing_inert: !document.getElementById("browse").checkVisibility() || document.elementFromPoint(5, 5)?.closest("#hs-sheet, html") !== null }; }));
    out.sections_open = await sections();
    let esc = 0; for (let i = 0; i < 30; i++) { await page.keyboard.press("Tab"); if (await page.evaluate(() => { const a = document.activeElement; return !!a && a !== document.body && !document.getElementById("hs-sheet").contains(a); })) esc++; }
    sh.tab_escapes = esc;
    await page.keyboard.press("Escape"); await page.waitForTimeout(300);
    Object.assign(sh, await page.evaluate((c) => ({ esc_closed: !document.getElementById("hs-sheet").open, focus_back: document.activeElement?.dataset?.addr === c }), inp.contract));
    await page.emulateMedia({ reducedMotion: "reduce" }); await page.click("#sheet-resume"); await until(sheetOpen, 5000);
    sh.anim_reduced = await page.evaluate(() => getComputedStyle(document.getElementById("hs-sheet")).animationName);
    await page.click("#sheet-close"); await page.waitForTimeout(300);
    Object.assign(sh, await page.evaluate(() => ({ close_btn_closed: !document.getElementById("hs-sheet").open, focus_back_resume: document.activeElement?.id === "sheet-resume" })));
    await page.emulateMedia({ reducedMotion: "no-preference" }); await page.click("#sheet-resume"); await until(sheetOpen, 5000);
    out.sheet = sh;
    out.hal_state = await attr("#hal-status", "data-state"); out.hal_sha = await attr("#hal-status", "data-sha"); out.hal_text = await text("#hal-status");
    await shot("01-instance-hal");
    // tampered local copy must be refused, then restore from the pinned source
    await page.setInputFiles("#hal-file", inp.tampered);
    await page.locator("#hal-status[data-state='error']").waitFor({ timeout: 30000 }).catch(() => {});
    out.hal_tamper_state = await attr("#hal-status", "data-state"); out.hal_tamper_err = (await text("#hal-err")) || (await text("#hal-status"));
    await shot("02-hal-tampered-refused");
    await page.click("#hal-fetch"); await page.locator("#hal-status[data-state='ok']").waitFor({ timeout: 60000 }).catch(() => {});
    out.hal_restored = await attr("#hal-status", "data-state");
    // info modal on every step: the sheet's component headers (always visible, also for locked components)
    // inline placement: every sheet info button lives in a component body, beside its field (heading, legend, label or button row), never in a header
    out.info_place = await page.evaluate(() => [...document.querySelectorAll("#hs-sheet button.info[data-info]")].map((b) => ({ id: b.dataset.info, head: !!b.closest(".comp-head"),
      sec: b.closest("section.comp")?.dataset.sec || "", ctx: (b.closest("h3,legend,label,.row,.danger") || {}).tagName || "" })));
    // Advanced disclosure: the raw-key option is collapsed by default, warns, and opens only on request
    out.adv = await page.evaluate(() => { const d = document.getElementById("adv-rawkey"); return { exists: !!d, open: d?.open, key_inside: !!d?.querySelector("#key") && !!d?.querySelector("#key-use"),
      warn: d?.querySelector(".danger")?.textContent.slice(0, 60) || "", }; });
    out.adv.key_visible = await page.locator("#key").isVisible();
    await page.click("#adv-rawkey > summary"); out.adv.key_visible_after_open = await page.locator("#key").isVisible(); await shot("03b-advanced-rawkey");
    const btns = page.locator("#hs-sheet button.info[data-info]"); const nb = await btns.count(); out.modals += nb;
    // buttons in still-locked components are revealed in the harness only (the user sees them once the component unlocks)
    for (let i = 0; i < nb; i++) { const b = btns.nth(i); if (!(await b.isVisible())) await b.evaluate((x) => { for (let e = x; e; e = e.parentElement) { if (e.hidden && e.classList.contains("comp-body")) { e.hidden = false; e.dataset.e2eReveal = "1"; } if (e.tagName === "DETAILS") e.open = true; } });
      if (i === 1) { await b.click(); await shot("03-info-modal-eligibility"); await page.click("#info-close"); } if (await infoOk(b)) out.modals_ok++; }
    await page.evaluate(() => { document.querySelectorAll("[data-e2e-reveal]").forEach((e) => { e.hidden = true; delete e.dataset.e2eReveal; }); document.getElementById("adv-rawkey").open = false; });
    out.sheet_open_after_modals = await sheetOpen();
    // eligibility: address from the EIP-1193 provider, lookup local; no request may carry the address
    const tA = Date.now(); await page.click("#mm-connect");
    await until(async () => (await attr("#elig-out", "data-notes")) !== null || (await text("#elig-err")), 30000);
    const id = inp.evm_addr.toLowerCase().replace(/^0x/, ""); const win = reqs.filter((r) => r.t >= tA);
    out.lookup_requests = win.length; out.leak = win.filter((r) => (r.url + r.body).toLowerCase().includes(id)).map((r) => r.url.slice(0, 100));
    out.elig_notes = Number(await attr("#elig-out", "data-notes")); out.elig_text = await text("#elig-out");
    out.reward_mode = await page.locator("input[name=reward-mode][value=ephemeral]").isChecked();
    await shot("04-eligible"); out.sections_eligible = await sections();
    last = null; await page.click("#derive");
    await until(async () => (await attr("#summary", "data-counts")) || (await text("#reg-err")), 600000);
    if (await text("#reg-err")) throw new Error("derive: " + (await text("#reg-err")));
    await saveLast(inp.out_derive); out.reward_addr = await text("#reward-addr"); out.sections_derived = await sections();
    // fix2: reward address locked visibly (radios disabled, inline note), a forced change raises no error
    out.reward_lock = await page.evaluate(() => { const rs = [...document.querySelectorAll("input[name=reward-mode]")]; const k = rs.find((r) => r.value === "keplr");
      k.disabled = false; k.click(); k.dispatchEvent(new Event("change")); return { disabled_before: true, lock_visible: !document.getElementById("reward-lock").hidden, lock_text: document.getElementById("reward-lock").textContent }; });
    await page.waitForTimeout(400);
    out.param_chips = await page.evaluate(() => document.querySelectorAll("#inst-list td .chips .chip").length);
    out.reward_lock.err = await text("#reward-err"); out.reward_lock.addr_same = (await text("#reward-addr")) === out.reward_addr;
    out.reward_lock.disabled_after = await page.evaluate(() => [...document.querySelectorAll("input[name=reward-mode]")].every((r) => r.disabled));
    await page.check("input[name=reward-mode][value=ephemeral]", { force: true }).catch(() => {});
    // fix2: copy chip in the sheet title
    out.copy = await page.evaluate(() => { const b = document.querySelector("#sheet-hs button.chip"); return b ? { text: b.textContent, title: b.title, aria: b.getAttribute("aria-label") } : null; });
    await page.click("#sheet-hs button.chip"); await until(async () => !!(await text("#toast")), 3000);
    out.copy.toast = await text("#toast"); out.copy.toast_live = await attr("#toast", "aria-live"); out.copy.copied = await attr("#sheet-hs button.chip", "data-copied");
    await page.locator(".sheet-head").screenshot({ path: `${inp.shots}/fix2-sheet-header.png` }).catch(() => {});
    const d0 = dls.length; out.downloads_before_register = dls.slice();
    const n0 = await logCount(/registered all notes/g); last = null; await page.click("#register");
    await until(async () => (await logCount(/registered all notes/g)) > n0 || (await text("#reg-err")), 900000);
    if (await text("#reg-err")) throw new Error("register: " + (await text("#reg-err")));
    await page.waitForTimeout(1500); out.downloads_on_register = dls.length - d0;
    await expand("save"); out.save_label = await text("#save"); out.save_note = await text("#save-note");
    await page.click("#save"); await saveLast(inp.out); out.downloads = dls.slice(); out.counts = JSON.parse((await attr("#summary", "data-counts")) || "{}");
    await shot("05-registered"); out.sections_registered = await sections();
    out.privacy = await page.evaluate(() => ({ ...document.getElementById("privacy").dataset })); out.reg_privacy = await text("#reg-privacy");
    for (const m of ["light", "dark"]) { await page.emulateMedia({ colorScheme: m }); await page.waitForTimeout(300); await page.locator("#reg-privacy").scrollIntoViewIfNeeded(); await vshot(`azure-register-${m}`); }
    await page.emulateMedia({ colorScheme: "light" });
  } else if (phase === "v2b") {
    await openHs(inp.contract);
    await page.setInputFiles("#nf", inp.notes); await page.click("#load");
    await until(async () => /loaded \d+ notes|ERROR/.test(await text("#log")) || (await text("#reg-err")), 60000);
    out.sections_loaded = await sections();
    // wallet (ephemeral default) must be confirmed before the claim component unlocks
    out.claim_locked_before_dest = (await attr("#sec-claim", "data-status")) === "locked";
    await page.click("#dest-ok"); await page.locator("#sec-claim[data-status='current']").waitFor({ timeout: 10000 });
    out.sections_dest = await sections();
    // anchor check lives in the claim component (unlocked after the destination is confirmed)
    await page.click("#anchor-check"); await page.waitForTimeout(1500); out.anchor = await text("#anchor-out");
    out.anchor_reason = await page.evaluate(() => ({ ...document.getElementById("anchor-reason").dataset, text: document.getElementById("anchor-reason").textContent,
      chain: [...document.querySelectorAll("#claim-chain .chip")].map((c) => c.dataset.step + ":" + c.dataset.state), picks: document.querySelectorAll("input[name=claim-note]").length,
      fwd_enabled: !document.querySelector("input[name=dest-mode][value=forward]").disabled, fwd_reason_hidden: document.getElementById("fwd-reason").hidden }));
    await shot("06-loaded-anchor");
    // R13 confirm gate: small privacy set -> claim disabled + confirmation shown; a forced click must not send anything
    await until(async () => (await attr("#privacy", "data-small")) !== null, 15000);
    out.gate = await page.evaluate(() => ({ ...document.getElementById("privacy").dataset, claim_disabled: document.getElementById("claim").disabled,
      ack_visible: !document.getElementById("privacy-ack-l").hidden, advice: document.getElementById("privacy-advice").textContent, relay_hidden: document.getElementById("delay-relay").hidden }));
    for (const m of ["light", "dark"]) { await page.emulateMedia({ colorScheme: m }); await page.waitForTimeout(300); await page.locator("#privacy").scrollIntoViewIfNeeded(); await vshot(`azure-privacy-confirm-${m}`); }
    await page.emulateMedia({ colorScheme: "light" });
    { const tG = Date.now(), n0 = await logCount(/submitted|proof|proving/gi);
      await page.evaluate(() => { const b = document.getElementById("claim"); b.disabled = false; b.click(); });
      await until(async () => /privacy set/.test(await text("#claim-err")), 15000); await page.waitForTimeout(1500);
      out.gate.forced_err = await text("#claim-err"); out.gate.forced_posts = reqs.filter((r) => r.t >= tG && r.body && !/"smart"|registration_tree|direct_claim/.test(r.body) && /tx|claim|relay/i.test(r.url)).map((r) => r.url.slice(0, 100));
      out.gate.disabled_after = await page.evaluate(() => document.getElementById("claim").disabled); out.gate.log_delta = (await logCount(/submitted|proof|proving/gi)) - n0; }
    await page.check("#privacy-ack"); out.gate.enabled_after_ack = await page.evaluate(() => !document.getElementById("claim").disabled);
    const runClaim = async (name) => {
      const n0 = await logCount(/claim run finished/g), e0 = await text("#claim-err"); last = null; const t1 = Date.now();
      await pickOne(); await page.click("#claim");
      await until(async () => (await logCount(/claim run finished/g)) > n0 || ((await text("#claim-err")) && (await text("#claim-err")) !== e0), 1800000);
      out[name + "_ms"] = Date.now() - t1; out[name + "_err"] = await text("#claim-err"); out[name + "_out"] = await text("#claim-out");
      await shot(name);
    };
    const pickOne = () => page.evaluate(() => { const cs = [...document.querySelectorAll("input[name=claim-note]")]; cs.forEach((c, i) => { c.checked = i === 0; c.dispatchEvent(new Event("change")); }); return cs.length; });
    await runClaim("07-claim-ephemeral");
    await expand("wallet"); await page.check("input[name=dest-mode][value=forward]"); await page.waitForTimeout(300);
    out.claim_relocked_on_dest_change = (await attr("#sec-claim", "data-status")) === "locked";
    out.channels = await page.locator("#fwd-channel option").allTextContents();
    out.zafu_detect = await text("#zafu-detect");
    if (!zx) throw new Error("forward-zafu needs the real Zafu extension (zafu_ext)");
    await page.click("#zafu-connect");
    await page.locator("#zafu-detect[data-state='connected'], #zafu-detect[data-state='error']").waitFor({ timeout: 90000 });
    { const crypto = await import("crypto"); const h = crypto.createHash("sha256").update(fs.realpathSync(inp.zafu_ext)).digest("hex").slice(0, 32);
      out.zafu_ext_id = zx.extId; out.zafu_ext_id_unpacked = [...h].map((c) => "abcdefghijklmnop"[parseInt(c, 16)]).join("") === zx.extId; }
    out.zafu_state = await attr("#zafu-detect", "data-state"); out.zafu_text = await text("#zafu-detect"); out.zafu_approvals = zx.approvals;
    await shot("08a-zafu-connected"); await page.click("#dest-ok"); await page.locator("#sec-claim[data-status='current']").waitFor({ timeout: 10000 });
    await runClaim("08-forward-zafu"); out.fwd_preview = await text("#fwd-preview");
    const z1 = await page.inputValue("#recv"); pen.list.push(z1); await expand("wallet");
    for (let i = 0; i < 2; i++) { await page.click("#zafu"); await until(async () => (await page.inputValue("#recv")) !== pen.list[pen.list.length - 1] || (await text("#dst-err")), 30000); pen.list.push(await page.inputValue("#recv")); }
    out.zafu_book = await page.evaluate(() => JSON.parse(localStorage.getItem("terp-hs-zafu-indices-v1") || "[]"));
    out.zafu_addrs_distinct = new Set(pen.list).size === pen.list.length && pen.list.every((x) => /^penumbra1[02-9ac-hj-np-z]{134}$/.test(x));
    out.zafu_addr_prefixes = pen.list.map((x) => x.slice(0, 18));
    await page.click("#zafu-disconnect"); await page.waitForFunction(() => document.querySelector("#zafu-detect")?.getAttribute("data-state") !== "connected", null, { timeout: 10000 }).catch(() => {}); out.zafu_after_disconnect = await attr("#zafu-detect", "data-state");
    await expand("wallet"); await page.check("#recv-any"); await page.fill("#recv", inp.recv_b);
    await runClaim("09-forward-chain-b");
    await expand("wallet"); await page.fill("#recv", z1); // the Zafu address already used by claim 08: must be refused by the reuse guard
    await runClaim("10-reuse-refused"); out.reuse_err = out["10-reuse-refused_err"];
    out.privacy_end = await page.evaluate(() => ({ ...document.getElementById("privacy").dataset })); out.sections_end = await sections();
    await expand("save"); await page.click("#save"); await saveLast(inp.out);
    out.counts = JSON.parse((await attr("#summary", "data-counts")) || "{}");
  } else if (phase === "theme") {
    const rd = () => page.evaluate(() => { const de = document.documentElement, cs = getComputedStyle(de);
      return { mode: de.dataset.theme, source: de.dataset.themeSource, rejected: de.dataset.themeRejected || "", bg: cs.getPropertyValue("--hs-bg").trim(), surface: cs.getPropertyValue("--hs-surface").trim(),
        accent: cs.getPropertyValue("--hs-accent").trim(), radius: cs.getPropertyValue("--hs-radius").trim(), body_bg: getComputedStyle(document.body).backgroundColor,
        btn_bg: getComputedStyle(document.querySelector("#browse")).backgroundColor, css: document.getElementById("hs-theme")?.textContent || "", toggle: document.getElementById("theme-toggle")?.textContent,
        stored: localStorage.getItem("terp-hs-theme-mode-v1") }; });
    const tag = inp.tag;
    if (inp.fake_wallets) { await until(async () => (await attr("#zafu-wallet", "data-count")) !== null, 20000);
      out.wallet_options = await page.locator("#zafu-wallet option").allTextContents(); out.wallet_select_visible = (await attr("#zafu-wallet", "hidden")) === null;
      await page.evaluate(() => { const s = document.getElementById("zafu-wallet"); s.value = "1"; s.dispatchEvent(new Event("change")); }); // section may be collapsed before a headstash is chosen
      await until(async () => (await text("#zafu-detect")).includes("selected chrome-extension://b"), 5000); out.wallet_detect = await text("#zafu-detect"); }
    await page.emulateMedia({ colorScheme: "light" }); await page.waitForTimeout(300); out.light = await rd(); await shot(`${tag}-light`);
    await page.emulateMedia({ colorScheme: "dark" }); await page.waitForTimeout(300); out.dark = await rd(); await shot(`${tag}-dark`);
    out.log_rejects = ((await text("#log")).match(/theme from \w+ rejected[^\n]*/g) || []).slice(0, 3);
    if (inp.toggle) {
      await page.click("#theme-toggle"); await page.waitForTimeout(200); out.after_toggle = await rd();
      await page.reload(); await page.locator("body[data-ready='1'], body[data-error]").waitFor({ timeout: 180000 });
      out.after_reload = await rd(); await shot(`${tag}-toggled-light-under-dark-pref`);
      await page.click("#theme-toggle"); await page.waitForTimeout(200); out.after_toggle2 = await rd();
    }
  }
  out.ok = true;
} catch (e) { out.error = String(e.message || e).slice(0, 400); await shot("error").catch(() => {}); }
out.penumbra_addrs = pen.list.length; out.real_zafu = !!zx; out.log = await text("#log"); await (browser ? browser.close() : ctx.close()); console.log(JSON.stringify(out));
