// R13: bottom-sheet layout, progressive disclosure, privacy set + confirm gate, delayed-relay extension point.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as V from "../v2lib.js";
const html = readFileSync(new URL("../app.html", import.meta.url), "utf8");
const js = readFileSync(new URL("../app.js", import.meta.url), "utf8");
const landing = html.slice(html.indexOf('<main id="landing">'), html.indexOf("</main>"));
const sheet = html.slice(html.indexOf('<dialog id="hs-sheet"'), html.indexOf("</dialog>", html.indexOf('<dialog id="hs-sheet"')));

test("landing view holds only browse/lookup; every claim tool lives inside the sheet", () => {
  for (const id of ["code-ids", "browse", "inst-list", "inst-err"]) assert.ok(landing.includes(`id="${id}"`), id);
  const tools = ["hal-fetch", "mm-connect", "key", "nf", "load", "pass", "derive", "register", "save", "export", "reward-export", "dest-mode", "fwd-channel", "recv", "zafu-connect", "zafu-wallet", "anchor-check", "privacy", "privacy-ack", "claim-pick", "claim", "dest-ok", "log"];
  for (const id of tools) { assert.ok(!landing.includes(`id="${id}"`) && !landing.includes(`name="${id}"`), "landing leaks " + id); assert.ok(sheet.includes(`id="${id}"`) || sheet.includes(`name="${id}"`), "sheet lacks " + id); }
});
test("sheet is an accessible modal bottom sheet", () => {
  assert.match(sheet, /^<dialog id="hs-sheet" class="sheet" aria-modal="true" aria-labelledby="sheet-title">/);
  assert.ok(sheet.includes('id="sheet-close"') && /aria-label="Close[^"]*"/.test(sheet));
  assert.match(html, /@media \(prefers-reduced-motion:reduce\)\{dialog\.sheet\[open\]\{animation:none\}\}/);
  assert.match(html, /dialog\.sheet\{position:fixed;inset:auto 0 0 0/);           // anchored to the bottom edge
  assert.match(html, /@keyframes hs-up\{from\{transform:translateY\(100%\)\}/);     // slides up
  assert.match(js, /\$\("hs-sheet"\)\.showModal\(\)|d\.showModal\(\)/);           // native modal: focus containment + Esc
  assert.match(html, /@media \(max-width:600px\)/);
});
test("components in flow order, each with toggle and collapsed body; info buttons inline beside their fields", () => {
  const secs = [...sheet.matchAll(/<section class="comp" id="sec-([a-z]+)" data-sec="\1" data-status="locked">/g)].map((m) => m[1]);
  assert.deepEqual(secs, ["eligibility", "register", "save", "wallet", "claim"]);
  for (const s of secs) {
    assert.ok(sheet.includes(`id="sec-${s}-toggle" aria-controls="sec-${s}-body" aria-expanded="false"`), s);
    assert.ok(sheet.includes(`<div class="comp-body" id="sec-${s}-body" hidden>`), s);
  }
  // info buttons sit inline beside the field they explain: none left bunched in the component headers
  const heads = [...sheet.matchAll(/<div class="comp-head">(.*?)<\/div>/g)].map((m) => [...m[1].matchAll(/data-info="([a-z]+)"/g)].length);
  assert.deepEqual(heads, [0, 0, 0, 0, 0]);
  const bodyOf = (s) => { const i = sheet.indexOf(`<div class="comp-body" id="sec-${s}-body"`); return sheet.slice(i, sheet.indexOf("</section>", i)); };
  const inl = { eligibility: ["hal", "eligibility", "rawkey", "notesfile"], register: ["reward", "passphrase", "register"], save: ["export"], wallet: ["destination"], claim: ["anchor", "privacy", "claim"] };
  for (const [s, ids] of Object.entries(inl)) assert.deepEqual([...bodyOf(s).matchAll(/data-info="([a-z]+)"/g)].map((x) => x[1]), ids, s);
  // each one is adjacent to its field: inside the heading, legend, label or button row it explains
  for (const [ctx, id] of [["<h3>Eligibility set (HAL) ", "hal"], ["<h3>Check your address", "eligibility"], ["<legend>Reward address ", "reward"], ['<label for="pass">', "passphrase"], ['<button id="register" disabled>Register notes</button>', "register"], ["<legend>Claim to ", "destination"], ['<button id="anchor-check">', "anchor"], ['<h3 id="privacy-h">', "privacy"], ['<button id="claim" disabled>Claim</button>', "claim"]]) {
    const i = sheet.indexOf(ctx); assert.ok(i >= 0, ctx); assert.ok(sheet.slice(i, i + 260).includes(`data-info="${id}"`), id);
  }
});
test("privacy set = registered − claimed from registration_tree + direct_claim", () => {
  const p = V.privacySet({ count: 43, min_anonymity_set: 2, latest_anchor: { root: "x", leaf_count: 40 } }, { claimed: 3 });
  assert.deepEqual(p, { registered: 43, claimed: 3, set: 40, anchored: 40, threshold: 64, small: true });
  assert.equal(V.privacySet({ count: "1000", min_anonymity_set: 100 }, { claimed: "10" }).threshold, 400);
  assert.equal(V.privacySet({ count: 1000, min_anonymity_set: 100 }, { claimed: 10 }).small, false);
  assert.equal(V.privacySet({ count: 64 }, { claimed: 0 }).small, false);
  assert.equal(V.privacySet({ count: 64 }, { claimed: 1 }).small, true);
  assert.equal(V.privacySet({ count: 5 }, {}).claimed, 0);
  for (const [rt, dc] of [[null, {}], [{ count: 1 }, null], [{ count: -1 }, {}], [{ count: 1.5 }, {}], [{ count: "1e3" }, {}], [{ count: 2 }, { claimed: 3 }], [{}, {}]]) assert.throws(() => V.privacySet(rt, dc));
  assert.match(V.privacyAdvice(V.privacySet({ count: 10 }, { claimed: 9 })), /^Only 1 registered note is still unclaimed.*at least 64/);
  assert.match(V.privacyAdvice(V.privacySet({ count: 100 }, { claimed: 0 })), /^100 registered notes .*suggested minimum 64/);
});
test("confirm gate: small or unknown set blocks a claim until the user confirms", () => {
  const small = V.privacySet({ count: 10 }, { claimed: 0 }), big = V.privacySet({ count: 500 }, { claimed: 0 });
  assert.equal(V.privacyGateOpen(small, false), false); assert.equal(V.privacyGateOpen(small, true), true);
  assert.equal(V.privacyGateOpen(big, false), true); assert.equal(V.privacyGateOpen(null, false), false); assert.equal(V.privacyGateOpen(null, true), true);
  assert.match(js, /if \(!V\.privacyGateOpen\(priv, \$\("privacy-ack"\)\.checked\)\) throw new Error/);   // enforced in claim(), not only by the disabled button
  assert.match(js, /\$\("claim"\)\.disabled = !\(claimable && V\.privacyGateOpen\(priv, \$\("privacy-ack"\)\.checked\)\)/);
});
test("delayed-claim relay: labelled extension point only, no service shipped", () => {
  assert.deepEqual(V.DELAYED_CLAIM_RELAYS, []); assert.ok(Object.isFrozen(V.DELAYED_CLAIM_RELAYS)); assert.deepEqual(V.delayedRelays(), []);
  const ok = { id: "r", name: "R", origin: "https://relay.example", submit: async () => ({}), status: async () => ({}) };
  assert.equal(V.delayedRelays([ok, { ...ok, origin: "http://x" }, { ...ok, submit: 1 }, null]).length, 1);
  assert.ok(sheet.includes('<div id="delay-relay" data-extension="delayed-claim-relay" data-count="0" hidden></div>'));
  assert.ok(!/relay\.example|fetch\([^)]*delay/i.test(js));
});
test("Azure Calcium is the default theme and matches frontend/themes/azure-calcium.json", () => {
  const f = JSON.parse(readFileSync(new URL("../themes/azure-calcium.json", import.meta.url), "utf8"));
  assert.equal(f.name, "Azure Calcium"); assert.deepEqual(f.theme, JSON.parse(JSON.stringify(V.AZURE_CALCIUM)));
  assert.equal(V.AZURE_CALCIUM.light.bg, "#EFF7FF"); assert.equal(V.AZURE_CALCIUM.dark.accent, "#76BBFC");
});
test("raw EVM key lives only under a collapsed Advanced disclosure with a strong warning", () => {
  const i = sheet.indexOf('<details id="adv-rawkey" class="adv">'); assert.ok(i > 0); const d = sheet.slice(i, sheet.indexOf("</details>", i));
  assert.ok(!/<details id="adv-rawkey"[^>]*\bopen\b/.test(sheet), "collapsed by default");
  assert.match(d, /<summary>Advanced<\/summary>/); assert.match(d, /class="danger" role="note"><b>Warning: do not paste a private key/);
  for (const id of ['id="key"', 'id="key-use"', 'data-info="rawkey"']) assert.ok(d.includes(id), id);
  assert.equal(sheet.split('id="key"').length, 2, "single key input");
});
test("system font stack, monospace for addresses, tabular numbers, 44px targets, no external fonts, colours only via --hs-*", () => {
  assert.ok(html.includes('--hs-sans:ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI Variable","Segoe UI",Inter,Roboto,"Helvetica Neue",Arial,sans-serif'));
  assert.ok(html.includes("--hs-mono:ui-monospace,SFMono-Regular,Menlo,monospace") && html.includes("font-variant-numeric:tabular-nums"));
  assert.match(html, /button\{[^}]*min-height:44px/); assert.match(html, /input,select\{[^}]*min-height:44px/); assert.match(html, /button\.info::after\{content:"";position:absolute;inset:-8px\}/);
  assert.ok(!/@import|@font-face|url\(\s*["']?https?:/i.test(html), "no external fetch");
  const css = html.slice(0, html.indexOf("</style>")); assert.deepEqual(css.match(/#[0-9a-f]{3,8}\b(?![-\w])/gi) || [], [], "colours only via --hs-*");
});
