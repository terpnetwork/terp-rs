// fix2: chips, copy chips, single download, forward reason, anchor reason with live numbers, locked reward address, claim chain.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as V from "../v2lib.js";
const html = readFileSync(new URL("../app.html", import.meta.url), "utf8");
const js = readFileSync(new URL("../app.js", import.meta.url), "utf8");

test("shortAddr truncates headstash addresses (terp1yyca…jf8) and passes short strings", () => {
  assert.equal(V.shortAddr("terp1yyca08xqdgvjz0psg56z67ejh9xms6l436u8y58m82npdqqhmmtqy30jf8"), "terp1yyca…jf8");
  assert.equal(V.shortAddr("abc"), "abc"); assert.equal(V.shortAddr(null), "");
});
test("fmtAmount shows micro denoms in display units", () => {
  assert.equal(V.fmtAmount(1000000000, "uterp"), "1,000 TERP"); assert.equal(V.fmtAmount(500000, "uterp"), "0.5 TERP");
  assert.equal(V.fmtAmount(7, "factory/x/y"), "7 factory/x/y");
});
test("anchorStatus: blocked by the anonymity set with live numbers", () => {
  const s = V.anchorStatus({ count: 1, min_anonymity_set: 2, next_anchor_height: 1, latest_anchor: null }, "1470", 1);
  assert.equal(s.state, "waiting-set"); assert.equal(s.need, 1); assert.equal(s.height, 1470);
  assert.match(s.reason, /1 of 2 registrations/); assert.match(s.eta, /1 more note is registered/);
});
test("anchorStatus: due / waiting-height / accepted / stale", () => {
  assert.equal(V.anchorStatus({ count: 2, min_anonymity_set: 2, next_anchor_height: 1 }, 5).state, "due");
  const w = V.anchorStatus({ count: 2, min_anonymity_set: 2, next_anchor_height: 10 }, 7); assert.equal(w.state, "waiting-height"); assert.match(w.reason, /block 10 \(now 7\)/);
  assert.equal(V.anchorStatus({ count: 3, min_anonymity_set: 2, latest_anchor: { leaf_count: 3 } }, 9).state, "accepted");
  assert.equal(V.anchorStatus({ count: 4, min_anonymity_set: 2, next_anchor_height: 1, latest_anchor: { leaf_count: 3 } }, 9).state, "stale");
  assert.throws(() => V.anchorStatus(null));
});
test("claimChain: Registered -> Anchor accepted -> Claim with exactly one current step", () => {
  const st = (n, a) => V.claimChain(n, a).map((x) => x.state).join(" ");
  assert.equal(st([{ st: "new" }], null), "current todo todo");
  assert.equal(st([{ st: "registered" }], { accepted: false }), "done current todo");
  assert.equal(st([{ st: "registered" }], { accepted: true }), "done done current");
  assert.equal(st([{ st: "spent" }], { accepted: true }), "done done done");
  assert.deepEqual(V.claimChain([], null).map((x) => x.label), ["Registered", "Anchor accepted", "Claim"]);
});
test("copy chips: accessible label, full address in title, toast is an aria-live status", () => {
  assert.match(js, /setAttribute\("aria-label", `Copy address \$\{addr\}`\)/); assert.match(js, /title: addr/);
  assert.match(html, /<div id="toast" role="status" aria-live="polite"><\/div>/);
  assert.match(js, /\$\("sheet-hs"\)\.replaceChildren\(copyChip\(a\)\)/); assert.match(js, /el\("br"\), copyChip\(a\)/);
});
test("parameters and privacy set render as chips, not raw text", () => {
  assert.match(js, /paramsDropdown\(i, root\)/); assert.ok(!/Privacy set now \$\{p\.set\}/.test(js));
  assert.match(html, /<div id="reg-privacy" class="chips" role="status"><\/div>/); assert.match(html, /\.chip\{/);
  for (const k of ["min set", "cutoff", "denom", "registered", "claimed"]) assert.ok(js.includes(`chip("${k}"`), k);
});
test("notes file downloads once; later saves are explicit and differently named", () => {
  const reg = js.slice(js.indexOf("async function register()"), js.indexOf("async function anchorCheck()"));
  assert.ok(!/await save\(\)/.test(reg), "register must not re-download"); assert.match(reg, /markDirty\("registered"\)/);
  const cl = js.slice(js.indexOf("async function claimRun()"), js.indexOf("async function exportWallets()"));
  assert.ok(!/await save\(\)/.test(cl), "claim must not re-download");
  assert.match(js, /Save updated notes \(now marked \$\{dirty\}\)/); assert.match(js, /V\.notesFileName\(chash, state && state\.identity, dirty\)/);
});
test("forward option shows an inline reason when no channel is allowlisted", () => {
  assert.match(html, /value="forward" disabled>/); assert.match(html, /id="fwd-reason" class="lock" role="status"/);
  assert.match(js, /allowlists no IBC forward channel/);
});
test("reward address is locked visibly after derive, never an error", () => {
  assert.ok(!/reward address is fixed once notes are derived/.test(js));
  assert.match(html, /id="reward-lock" class="lock" hidden>Locked:/); assert.match(js, /r\.disabled = l/);
});
test("claim picker replaces the opaque max-notes count", () => {
  assert.ok(!html.includes('id="claim-max"')); assert.match(html, /<fieldset id="claim-pick"><legend>Notes to claim in this run<\/legend>/);
  assert.match(js, /name: "claim-note"/); assert.match(js, /select at least one note to claim/);
});
test("anchor reason box is live and shows the dependency chain", () => {
  assert.match(html, /<ol id="claim-chain" class="steps" aria-label="Claim steps"><\/ol>/); assert.match(html, /id="anchor-reason" class="reason" role="status" aria-live="polite"/);
  assert.match(js, /chip\("min anonymity set", s\.min\)/); assert.match(js, /re-checks every \$\{ANCHOR_POLL_S\} s/);
});
