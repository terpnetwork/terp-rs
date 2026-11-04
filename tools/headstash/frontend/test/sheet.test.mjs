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
  const tools = ["hal-fetch", "mm-connect", "key", "nf", "load", "pass", "derive", "register", "save", "export", "reward-export", "dest-mode", "fwd-channel", "recv", "zafu-connect", "zafu-wallet", "anchor-check", "privacy", "privacy-ack", "claim-max", "claim", "dest-ok", "log"];
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
test("components in flow order, each with toggle, collapsed body, and info buttons in the always-visible header", () => {
  const secs = [...sheet.matchAll(/<section class="comp" id="sec-([a-z]+)" data-sec="\1" data-status="locked">/g)].map((m) => m[1]);
  assert.deepEqual(secs, ["eligibility", "register", "save", "wallet", "claim"]);
  for (const s of secs) {
    assert.ok(sheet.includes(`id="sec-${s}-toggle" aria-controls="sec-${s}-body" aria-expanded="false"`), s);
    assert.ok(sheet.includes(`<div class="comp-body" id="sec-${s}-body" hidden>`), s);
  }
  const heads = [...sheet.matchAll(/<div class="comp-head">(.*?)<\/div>/g)].map((m) => [...m[1].matchAll(/data-info="([a-z]+)"/g)].map((x) => x[1]));
  assert.deepEqual(heads, [["hal", "eligibility"], ["reward", "register"], ["export"], ["destination"], ["anchor", "privacy", "claim"]]);
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
