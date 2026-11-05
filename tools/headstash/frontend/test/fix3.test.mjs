import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const here = (f) => new URL("../" + f, import.meta.url);
const V = await import(here("v2lib.js"));
const js = readFileSync(here("app.js"), "utf8"), html = readFileSync(here("app.html"), "utf8");

test("fix4 filename: headstash-notes-<label-or-contract8>-<0x…6>-<status>.json", () => {
  const h = "a1b2c3d4e5f6", id = "f3e03d49aa00bb11223344556677889900aabbcc";
  assert.equal(V.notesFileName({ contractHash: h, identity: id }), "headstash-notes-a1b2c3d4-0xf3e03d-new.json");
  assert.equal(V.notesFileName({ label: "Resin Drop #2", contractHash: h, identity: "0x" + id, status: "registered" }), "headstash-notes-resin-drop-2-0xf3e03d-registered.json");
  assert.equal(V.notesFileName({ label: "", contractHash: h, identity: id, status: "claimed" }), "headstash-notes-a1b2c3d4-0xf3e03d-claimed.json");
  assert.notEqual(V.notesFileName({ contractHash: "ffff0000", identity: id }), V.notesFileName({ contractHash: h, identity: id }));
  assert.notEqual(V.notesFileName({ contractHash: h, identity: "254768d4" + id.slice(8) }), V.notesFileName({ contractHash: h, identity: id }));
  assert.ok(V.notesFileName({ label: "x".repeat(80), contractHash: h, identity: id }).length < 60);
  assert.match(js, /function saveName\(\) \{ return V\.notesFileName\(\{ label/); assert.match(js, /chash = hex\(await sha256\(te\.encode\(a\)\)\)/);
});
test("fix3 load refuses a notes file for another Headstash (no auto-switch)", () => {
  assert.throws(() => V.checkNotesContract({ contract: "terp1aaaaaaaaaaaaaaaaaaaa" }, "terp1bbbbbbbbbbbbbbbbbbbb"), /belongs to Headstash terp1aaaa.*not the open one/);
  assert.throws(() => V.checkNotesContract({}, "terp1b"), /no Headstash contract/);
  assert.equal(V.checkNotesContract({ contract: "terp1x" }, "terp1x"), true);
  assert.match(js, /V\.checkNotesContract\(o, inst \? inst\.addr : null\)/); assert.ok(!/await selectInstance\(o\.contract\)/.test(js));
});
test("fix3 P0 chain-derived gate: note_list next marks notes registered; claim needs accepted anchor", () => {
  const notes = [{ st: "new" }, { st: "new" }];
  assert.equal(V.syncRegistered(notes, { n: 2, next: 1 }), 1); assert.equal(notes[0].st, "registered"); assert.equal(notes[1].st, "new"); assert.equal(notes[1].chain, "unregistered");
  assert.equal(V.syncRegistered([{ st: "new" }], null), 0);
  const one = [{ st: "new" }]; V.syncRegistered(one, { n: 1, next: 1 });
  const g1 = V.claimGate(one, { accepted: false, count: 1, reason: "Anchor blocked: 1 of 2." }); assert.equal(g1.ok, false); assert.match(g1.reason, /Only 1 registration exists/);
  assert.equal(V.claimGate(one, { accepted: true, count: 2, reason: "Anchor accepted" }).ok, true);
  assert.equal(V.claimGate([{ st: "new", chain: "unregistered" }], { accepted: true }).ok, false);
  assert.equal(V.claimGate([{ st: "registered" }], { accepted: true }).ok, false, "file-only status is not enough");
  assert.match(js, /note_list: \{ identity: state\.identity \}/);
  assert.match(js, /V\.claimGate\(state \? state\.notes : \[\], anc\)/);
  assert.match(js, /gate\.ok && V\.privacyGateOpen/);
  for (const f of ["async function load", "async function register", "async function privacyRefresh"]) assert.match(js.slice(js.indexOf(f), js.indexOf(f) + 2500), /chainSync\(\)/, f);
  assert.match(js, /already registered\/i\.test\(e\.message\)\) \{ await chainSync\(\)/);
});
test("fix3 MetaMask: accountsChanged/chainChanged refresh, connect + disconnect with revoke fallback", () => {
  assert.match(js, /walletScope\.on\(eth, "accountsChanged", onAccounts\)/); assert.match(js, /walletScope\.on\(eth, "chainChanged", onChain\)/);
  assert.match(js, /wallet_revokePermissions/); assert.match(js, /disconnecting locally/);
  assert.ok(html.includes('id="mm-disconnect"') && html.includes('id="mm-account"'));
  const on = js.slice(js.indexOf("async function onAccounts"), js.indexOf("function onChain"));
  assert.match(on, /if \(elig\) showElig\(\)/); assert.match(on, /clearNotes\(\)/); assert.match(on, /not saved yet/);
  // listener wiring: Scope with an EIP-1193 style emitter registers once and removes cleanly
  const ev = new Map(), eth = { on: (e, f) => ev.set(e, (ev.get(e) || 0) + 1), removeListener: (e) => ev.set(e, ev.get(e) - 1) };
  const s = new V.Scope(); s.on(eth, "accountsChanged", () => {}); s.on(eth, "chainChanged", () => {}); assert.deepEqual(s.size, { timers: 0, listeners: 2 });
  s.dispose(); assert.equal(ev.get("accountsChanged"), 0); assert.equal(ev.get("chainChanged"), 0);
});
test("fix3 instance switch: teardown disposes timers/listeners; N switches do not grow", () => {
  let live = 0, nid = 0; const api = { setInterval: () => (live++, ++nid), clearInterval: () => live-- };
  const tgt = { n: 0, addEventListener() { this.n++; }, removeEventListener() { this.n--; } };
  let scope = null;
  const switchTo = () => { if (scope) scope.dispose(); scope = new V.Scope(api); scope.interval(() => {}, 15000); scope.on(tgt, "x", () => {}); };
  for (let i = 0; i < 50; i++) switchTo();
  assert.equal(live, 1); assert.equal(tgt.n, 1); assert.deepEqual(scope.size, { timers: 1, listeners: 1 });
  assert.ok(!/setInterval\(/.test(js), "every timer goes through the instance Scope");
  assert.match(js, /ancTimer = instScope\.interval\(/); assert.ok(!/reload the page to switch/.test(js));
  const sel = js.slice(js.indexOf("async function selectInstance"), js.indexOf("async function selectInstance") + 600); assert.match(sel, /teardown\(\)/);
  const td = js.slice(js.indexOf("function teardown"), js.indexOf("function teardown") + 700); assert.match(td, /instScope\.dispose\(\)/); assert.match(td, /clearNotes\(\)/); assert.match(td, /anchorPoll\(\)/);
});
test("fix3 filters: tokens from bank balances + denom, creators; filters combine; searchable", () => {
  assert.deepEqual(V.instanceTokens([{ denom: "factory/terp1abc/pinecone", amount: "5" }, { denom: "uterp", amount: "1" }], "uterp"), ["factory/terp1abc/pinecone", "uterp"]);
  assert.equal(V.prettyDenom("factory/terp1abc/pinecone"), "pinecone"); assert.equal(V.prettyDenom("uterp"), "TERP"); assert.match(V.prettyDenom("ibc/27394FB092D2ECCD"), /^IBC 27394F…$/);
  const L = [{ addr: "a", creator: "c1", tokens: ["uterp"] }, { addr: "b", creator: "c2", tokens: ["uterp", "factory/x/pine"] }, { addr: "c", creator: "c1", tokens: ["factory/x/pine"] }];
  assert.deepEqual(V.filterInstances(L, {}).map((i) => i.addr), ["a", "b", "c"]);
  assert.deepEqual(V.filterInstances(L, { tokens: ["factory/x/pine"] }).map((i) => i.addr), ["b", "c"]);
  assert.deepEqual(V.filterInstances(L, { creators: ["c1"] }).map((i) => i.addr), ["a", "c"]);
  assert.deepEqual(V.filterInstances(L, { tokens: ["factory/x/pine"], creators: ["c1"] }).map((i) => i.addr), ["c"]);
  const opts = V.filterOptions(L, "tokens"); assert.deepEqual(opts.map((o) => o.label), ["pine", "TERP"]); assert.equal(opts[0].n, 2);
  assert.deepEqual(V.searchOptions(opts, "pin").map((o) => o.value), ["factory/x/pine"]);
  assert.match(js, /nullifer: \{ null: `0:\$\{n\.n\}` \}/); assert.match(js, /return ch \+ sp;/);
  assert.match(js, /if \(gen !== browseGen\) return;/); assert.match(js, /tb\.replaceChildren\(\.\.\.rows\); all = acc;/);
  assert.match(js, /\/cosmos\/bank\/v1beta1\/balances\/\$\{a\}/); assert.match(js, /creator: ci\.creator/);
  assert.ok(html.includes('id="filter-dlg"') && html.includes('id="filter-q"') && /\.flist\{max-height:50vh;overflow:auto/.test(html));
});
test("fix3 footer Details modal + layout: code-id form, critical chips, label/creator title", () => {
  const land = html.slice(html.indexOf('<main id="landing">'), html.indexOf("</main>"));
  assert.match(land, /<footer class="foot verify" id="verify"[^>]*>.*<button id="details-btn"/s); assert.ok(html.includes('id="sheet-details-btn"'));
  const dd = html.slice(html.indexOf('<dialog id="details-dlg"'), html.indexOf("</dialog>", html.indexOf('<dialog id="details-dlg"')));
  assert.ok(dd.includes('id="fe-addr"') && dd.includes('id="fe-ver"')); assert.ok(!html.slice(0, html.indexOf("<main")).includes('id="fe-addr"'));
  const dr = js.slice(js.indexOf("function detailsRender"), js.indexOf("function detailsOpen"));
  for (const k of ["Code id", "Admin", "Creator", "HAL URI", "HAL commitment", "sha256", "Configuration"]) assert.ok(dr.includes(k), k);
  assert.match(land, /<div id="code-form" class="pop"[^>]*hidden>.*id="code-ids"/s);
  assert.match(js, /paramsBtn\(\(\) => paramsOpen\(\{ \.\.\.i, code_id/); assert.ok(!/critChips|paramsDropdown/.test(js));
  assert.match(html, /<h2 id="sheet-title"><span id="sheet-label">Headstash<\/span> <span id="sheet-hs"><\/span><\/h2><div id="sheet-creator"/);
  assert.match(js, /\$\("sheet-label"\)\.textContent = ci\.label/); assert.match(js, /"creator ", copyChip\(ci\.creator\)/);
});
