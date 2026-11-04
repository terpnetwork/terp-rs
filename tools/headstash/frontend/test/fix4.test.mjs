// fix4: find toolbar, header/footer content, opt-in HAL fetch, kv tables, account card, Parameters modal, notes filename, bridge panel
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const here = (p) => new URL("../" + p, import.meta.url).pathname;
const V = await import(here("v2lib.js"));
const js = readFileSync(here("app.js"), "utf8"), html = readFileSync(here("app.html"), "utf8");
const land = html.slice(html.indexOf('<main id="landing">'), html.indexOf("</main>"));
const head = html.slice(html.indexOf("<header>"), html.indexOf("</header>"));

test("fix4 find toolbar: search, filters, code-id toggle and refresh on one grid row; code form is an overlay", () => {
  const bar = land.slice(land.indexOf('<div class="toolbar" id="filters"'), land.indexOf('<div class="toolbar-meta"'));
  const order = ["inst-q", "f-token", "f-creator", "code-toggle", "browse"].map((id) => bar.indexOf(`id="${id}"`));
  assert.ok(order.every((x, i) => x > 0 && (i === 0 || x > order[i - 1])), String(order));
  assert.match(html, /\.toolbar\{position:relative;display:grid;grid-template-columns:minmax\(0,1fr\) auto auto auto auto/);
  assert.match(html, /\.pop\{position:absolute;/); assert.match(bar, /<div id="code-form" class="pop" role="group"[^>]*hidden>/);
  assert.match(js, /\$\("code-toggle"\)\.onclick/); assert.match(js, /setAttribute\("aria-expanded", String\(o\)\)/);
  assert.deepEqual(V.filterInstances([{ addr: "terp1aa", label: "Resin" }, { addr: "terp1bb", label: "Kief", creator: "terp1zz" }], { q: "res" }).map((i) => i.addr), ["terp1aa"]);
  assert.deepEqual(V.filterInstances([{ addr: "terp1aa", label: "Resin" }, { addr: "terp1bb", label: "Kief", creator: "terp1zz" }], { q: "TERP1ZZ" }).map((i) => i.addr), ["terp1bb"]);
});
test("fix4 header describes headstashes; footer carries verifiability (chunks, shas, CIDs)", () => {
  assert.match(head, /A <b>Headstash<\/b> is a private airdrop/); assert.ok(!/sha256|chunk/i.test(head), "no verify text at the top");
  const f = land.slice(land.indexOf('<footer class="foot verify"'));
  assert.match(f, /chunks stored in a frontend contract/); assert.match(f, /sha256/); assert.match(f, /CID/); assert.match(f, /id="verify-kv"/);
  const vr = js.slice(js.indexOf("function verifyRender"), js.indexOf("function detailsOpen"));
  for (const k of ["Frontend contract", "On-chain chunks", "sha256 ${f.path}"]) assert.ok(vr.includes(k), k);
});
test("fix4 eligibility: header + HAL subheader, opt-in fetch only, kv source table", () => {
  assert.match(html, /<h3>Eligibility list <button class="info"[^>]*>i<\/button><\/h3>\n<p class="subh">Headstash Allocation List \(eligibility set\)<\/p>/);
  const sel = js.slice(js.indexOf("async function selectInstance"), js.indexOf("function fwdState"));
  assert.ok(!/halFetch\(/.test(sel), "selectInstance must not fetch the HAL"); assert.equal((js.match(/halFetch\(\)/g) || []).length, 1, "only the definition; the button is the sole caller");
  assert.match(sel, /kvTable\(\$\("hal-kv"\), hal \? \[\["Source", hal\.uri, 1\], \["sha256"/); assert.ok(!html.includes('id="hal-src"'));
  assert.match(js, /function eligPrompt\(\) \{ \$\("elig-out"\)\.textContent = `Fetch the eligibility list above/);
  const mm = js.slice(js.indexOf("async function mmConnect"), js.indexOf("async function keyUse"));
  assert.ok(!/verify the HAL first/.test(mm)); assert.match(mm, /if \(elig\) showElig\(\); else eligPrompt\(\);/);
  assert.match(html, /button\.kvv\{[^}]*border-radius:var\(--hs-r\)/); assert.match(html, /button\.kvv>span\{[^}]*text-overflow:ellipsis;white-space:nowrap/);
});
test("fix4 account card: fixed grid, both buttons always rendered (disabled not hidden), address copy + network", () => {
  const c = html.slice(html.indexOf('<div class="acct"'), html.indexOf("</div></div>", html.indexOf('class="acct-btns"')));
  for (const id of ["mm-addr", "mm-copy", "mm-account", "mm-net", "mm-connect", "mm-disconnect"]) assert.ok(c.includes(`id="${id}"`), id);
  assert.ok(!/id="mm-disconnect"[^>]*hidden/.test(c)); assert.match(html, /\.acct\{display:grid;grid-template-columns:minmax\(0,1fr\) 8\.5rem 9\.5rem/);
  const ui = js.slice(js.indexOf("function mmUi"), js.indexOf("function eligPrompt"));
  assert.match(ui, /\$\("mm-disconnect"\)\.hidden = false; \$\("mm-disconnect"\)\.disabled = !on;/); assert.match(ui, /chainName\(who\.chain\)/);
});
test("fix4 Parameters modal: two-column kv table for list rows and the open sheet", () => {
  assert.match(html, /<dialog id="params-dlg" class="modal"[^>]*>.*<table class="kv" id="params-kv"/);
  assert.match(js, /\$\("sheet-crit"\)\.replaceChildren\(paramsBtn\(/); assert.match(js, /kvTable\(\$\("params-kv"\), paramRows\(i, root\)\)/);
  assert.match(html, /table\.kv\{table-layout:fixed/);
});
test("fix4 notes identity check on load", () => {
  const o = { identity: "ab".repeat(20), eligible_address: "0x" + "ab".repeat(20) };
  assert.equal(V.checkNotesIdentity(o, null), true); assert.equal(V.checkNotesIdentity(o, "0x" + "AB".repeat(20)), true);
  assert.throws(() => V.checkNotesIdentity(o, "cd".repeat(20)), /is for eligible address 0xababab…abab, but MetaMask is connected as 0xcdcdcd/);
  assert.throws(() => V.checkNotesIdentity({ identity: "ab".repeat(20), eligible_address: "0x" + "cd".repeat(20) }, null), /inconsistent/);
  assert.equal(V.checkNotesIdentity({ identity: "ab".repeat(20) }, null), true, "pre-fix4 files stay loadable");
  assert.match(js, /eligible_address: "0x" \+ who\.id/); assert.match(js, /V\.checkNotesIdentity\(o, who && who\.kind === "eip1193" \? who\.id : null\)/);
});
test("fix4 post-claim bridge panel: wallet detect, install link when absent, reason when no channel, fresh address via IndexBook", () => {
  const b = html.slice(html.indexOf('<div id="bridge"'), html.indexOf('<p class="err" id="bridge-err"'));
  assert.match(b, /<a id="zafu-install" class="btn" href="https:\/\/zafu\.pro" target="_blank" rel="noopener noreferrer" hidden>/);
  assert.match(js, /if \(done\) \{ markDirty\("claimed"\); await bridgeShow\(mode\); \}/);
  const bs = js.slice(js.indexOf("async function bridgeShow"), js.indexOf("async function bridgeNewAddr"));
  assert.match(bs, /zafuDetect\(\)/); assert.match(bs, /\$\("zafu-install"\)\.hidden = !!pick/); assert.match(bs, /forward_channels is empty/);
  assert.match(js, /bridgeAddr = await zafuAddress\("bridge"\)/);
  const za = js.slice(js.indexOf("async function zafuAddress"), js.indexOf("async function receiverFor"));
  assert.match(za, /book\.allocate\(/); assert.match(za, /guard\.check\(a\.address, owner\)/); assert.match(za, /book\.record\(/);
});
