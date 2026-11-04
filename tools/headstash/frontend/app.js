// Headstash claim page v2 (R1-R9). Loaded by loader.html from the frontend contract; every file sha256-verified.
let C, V, hex, unhex, cat, b64, unb64, te, sha256;
const $ = (id) => document.getElementById(id);
let cfg, Z, STEPS, guard, state = null, inst = null, elig = null, who = null, rawKey = null, rewardW = null;
// R13 sheet + progressive disclosure state; R13 privacy set
let priv = null, saved = false, destOk = false, cur = null, opener = null; const expanded = new Set();
let browseGen = 0, instScope = null, walletScope = null, all = [], chash = ""; const filt = { tokens: new Set(), creators: new Set() };
const log = (...a) => { const l = $("log"); l.textContent += new Date().toISOString().slice(11, 19) + " " + a.join(" ") + "\n"; l.scrollTop = 1e9; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const errOut = (id, e) => { $(id).textContent = e ? (e.message || String(e)) : ""; if (e) log("ERROR", e.message || String(e)); };
const el = (tag, props = {}, ...kids) => { const e = document.createElement(tag); Object.assign(e, props); kids.forEach((k) => e.append(k)); return e; };
async function guarded(errId, fn) { errOut(errId, null); try { return await fn(); } catch (e) { errOut(errId, e); } finally { flow(); } }

// ---- chain IO (LCD + relay + HAL origin; CSP connect-src enforces this) ----
async function lcd(path) { const r = await fetch(cfg.rest + path); const j = await r.json().catch(() => ({})); if (!r.ok) throw new Error(j.message || "LCD " + r.status); return j; }
async function smartAt(addr, q) { return (await lcd(`/cosmwasm/wasm/v1/contract/${addr}/smart/${b64(te.encode(JSON.stringify(q)))}`)).data; }
const smart = (q) => smartAt(cfg.headstash, q);
async function grant(grantee, kind, body) {
  const r = await fetch(`${cfg.relay}/headstash/feegrant`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ grantee, [kind]: body }) });
  const j = await r.json().catch(() => ({})); if (r.status !== 200 || !j.granted) throw new Error("fee grant refused: " + JSON.stringify(j).slice(0, 200)); return j;
}
async function account(addr) {
  for (let i = 0; i < 30; i++) {
    const r = await fetch(`${cfg.rest}/cosmos/auth/v1beta1/accounts/${addr}`);
    if (r.ok) { const a = (await r.json()).account; const b = a.base_account || a; return { num: b.account_number, seq: b.sequence || "0" }; }
    await sleep(1000);
  }
  throw new Error("account not found after grant: " + addr);
}
async function wallet(priv) { const pub = C.pubkey(priv).compressed; return { priv, pub, addr: C.bech32Encode(cfg.hrp, C.ripemd160(await sha256(pub))) }; }
async function execTx(w, msg, gas, granter) {
  const { num, seq } = await account(w.addr);
  const exec = cat(C.pb.str(1, w.addr), C.pb.str(2, cfg.headstash), C.pb.bytes(3, te.encode(JSON.stringify(msg))));
  const body = C.pb.any(1, "/cosmwasm.wasm.v1.MsgExecuteContract", exec);
  const fee = Math.ceil(gas * cfg.gas_price);
  const signer = cat(C.pb.any(1, "/cosmos.crypto.secp256k1.PubKey", C.pb.bytes(1, w.pub)), C.pb.bytes(2, C.pb.bytes(1, C.pb.uint(1, 1))), C.pb.uint(3, seq));
  const auth = cat(C.pb.bytes(1, signer), C.pb.bytes(2, cat(C.pb.bytes(1, cat(C.pb.str(1, cfg.denom), C.pb.str(2, String(fee)))), C.pb.uint(2, gas), C.pb.str(4, granter))));
  const doc = cat(C.pb.bytes(1, body), C.pb.bytes(2, auth), C.pb.str(3, cfg.chain_id), C.pb.uint(4, num));
  const sig = await C.sign(w.priv, await sha256(doc));
  const raw = cat(C.pb.bytes(1, body), C.pb.bytes(2, auth), C.pb.bytes(3, cat(sig.r, sig.s)));
  const r = await fetch(`${cfg.rest}/cosmos/tx/v1beta1/txs`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ tx_bytes: b64(raw), mode: "BROADCAST_MODE_SYNC" }) });
  const tr = (await r.json()).tx_response || {};
  if (tr.code) return { code: tr.code, raw_log: tr.raw_log, txhash: tr.txhash };
  for (let i = 0; i < 60; i++) {
    await sleep(1000);
    const q = await fetch(`${cfg.rest}/cosmos/tx/v1beta1/txs/${tr.txhash}`);
    if (q.ok) { const t = (await q.json()).tx_response; return { code: t.code, raw_log: t.raw_log, txhash: t.txhash, gas_used: t.gas_used }; }
  }
  return { code: -1, raw_log: "not included after 60s", txhash: tr.txhash };
}
const rootHex = (r) => (r == null ? null : /^[0-9a-f]{64}$/i.test(r) ? r.toLowerCase() : hex(unb64(r)));
const listOf = (x, k) => (Array.isArray(x) ? x : (x && (x[k] || x.channels)) || []);

// ---- fix2: chips, copy chips, toast ----
const chip = (label, value, extra = {}) => { const c = el("span", { className: "chip", ...extra }); if (label) c.append(el("i", { textContent: label })); c.append(String(value)); return c; };
let toastT = 0;
function toast(msg) { const t = $("toast"); t.textContent = msg; clearTimeout(toastT); toastT = setTimeout(() => (t.textContent = ""), 2500); }
async function copyText(s) {
  try { await navigator.clipboard.writeText(s); return true; } catch {}
  const ta = el("textarea", { value: s }); ta.setAttribute("readonly", ""); ta.style.position = "fixed"; ta.style.opacity = "0"; document.body.appendChild(ta); ta.select();
  let ok = false; try { ok = document.execCommand("copy"); } catch {} ta.remove(); return ok;
}
function copyChip(addr) {
  const b = el("button", { type: "button", className: "chip copy", textContent: V.shortAddr(addr), title: addr });
  b.dataset.copy = addr; b.setAttribute("aria-label", `Copy address ${addr}`);
  b.onclick = async (e) => { e.stopPropagation(); const ok = await copyText(addr); b.dataset.copied = ok ? "1" : "0"; toast(ok ? `Copied ${V.shortAddr(addr)}` : "Copy failed: select the address manually"); };
  return b;
}
const amt = (x) => V.fmtAmount(x, cfg.denom);

// ---- R7: browse instances by code id ----
async function instanceInfo(a) {
  const ci = (await lcd(`/cosmwasm/wasm/v1/contract/${a}`)).contract_info || {};
  const [pause, hal, ch, camp, rt, dc] = await Promise.all([{ pause_status: {} }, { hal_source: {} }, { forward_channels: {} }, { epoch_campaign: {} }, { registration_tree: {} }, { direct_claim: {} }].map((q) => smartAt(a, q).catch(() => undefined)));
  const bal = await lcd(`/cosmos/bank/v1beta1/balances/${a}`).catch(() => ({ balances: [] }));
  return { addr: a, label: ci.label, admin: ci.admin, creator: ci.creator, code_id: ci.code_id, pause, hal, channels: listOf(ch, "channels"), camp, rt, dc, tokens: V.instanceTokens(bal.balances, cfg.denom), balances: bal.balances || [] };
}
let defaultHs = null;
function critChips(i) {
  const box = el("div", { className: "chips crit" }), c = i.camp || {}, r = c.registration || {};
  for (const x of c.amounts || []) box.append(chip("", `${amt(x.amount)} ×${x.cap}`));
  if (r.min_anonymity_set != null) box.append(chip("min set", r.min_anonymity_set));
  if (c.cutoff_height != null) box.append(chip("cutoff", `block ${c.cutoff_height}`));
  return box;
}
function paramsDropdown(i, root) { const d = el("details", { className: "params" }, el("summary", { textContent: "All parameters" }), paramChips(i, root)); return d; }
function paramChips(i, root) {
  const box = el("div", { className: "chips" }), c = i.camp || {}, r = c.registration || {};
  for (const x of c.amounts || []) box.append(chip("note", `${amt(x.amount)} ×${x.cap}`));
  if (cfg.denom) box.append(chip("denom", cfg.denom));
  if (r.min_anonymity_set != null) box.append(chip("min set", r.min_anonymity_set));
  if (c.cutoff_height != null) box.append(chip("cutoff", `block ${c.cutoff_height}`));
  if (i.rt) box.append(chip("registered", i.rt.count), chip("anchors", i.rt.anchor_count ?? (i.rt.latest_anchor ? 1 : 0)));
  if (i.dc) box.append(chip("claimed", i.dc.claimed ?? 0));
  const fc = i.channels ? i.channels.filter((x) => x.enabled).map((x) => x.channel_id) : null;
  if (i.tokens) box.append(chip("tokens", i.tokens.map(V.prettyDenom).join(" ") || "none", { title: i.tokens.join(" ") }));
  if (i.creator) box.append(chip("creator", V.shortAddr(i.creator), { title: i.creator }));
  box.append(chip("forward", fc ? fc.join(" ") || "none" : "?"), chip("HAL", i.hal ? "set" : "unset", i.hal ? { title: i.hal.uri } : {}), chip("admin", i.admin ? V.shortAddr(i.admin) : "none", i.admin ? { title: i.admin } : {}), chip("root", root ? root.slice(0, 10) + "…" : "?", root ? { title: root } : {}));
  return box;
}
async function browse() {
  return guarded("inst-err", async () => {
    const codes = V.parseNumList($("code-ids").value); if (!codes.length) throw new Error("enter at least one code id");
    const tb = $("inst-list").tBodies[0], gen = ++browseGen, rows = [], acc = []; let n = 0;
    for (const code of codes) {
      let key = null;
      do {
        const j = await lcd(V.codeContractsPath(code, key));
        for (const a of j.contracts || []) {
          const i = await instanceInfo(a).catch((e) => ({ addr: a, error: e.message }));
          const paused = i.pause && (i.pause.paused || i.pause.claims_paused) ? "paused" : i.pause ? "active" : "?";
          const root = rootHex(i.camp?.registration?.eligibility_root);
          const params = el("div", {}, chip("", i.error ? "error" : paused), critChips(i), paramsDropdown(i, root));
          const btn = el("button", { textContent: "Open" });
          btn.setAttribute("aria-label", "Open Headstash " + a); btn.setAttribute("aria-haspopup", "dialog"); btn.dataset.addr = a; btn.onclick = () => openInstance(a, btn);
          const who_ = el("span", { className: "muted sub" }); if (i.creator) who_.append("by ", copyChip(i.creator));
          const tr = el("tr", {}, el("td", {}, el("b", { textContent: i.label || "(no label)" }), el("br"), copyChip(a), el("br"), who_), el("td", { textContent: String(code) }), i.error ? el("td", { textContent: i.error }) : el("td", {}, params), el("td", {}, btn));
          tr.dataset.addr = a; tr.dataset.creator = i.creator || ""; tr.dataset.tokens = (i.tokens || []).join(" ");
          rows.push(tr); acc.push({ addr: a, creator: i.creator, admin: i.admin, tokens: i.tokens || [], label: i.label, row: tr });
          n++;
        }
        key = j.pagination && j.pagination.next_key;
      } while (key);
    }
    if (gen !== browseGen) return; // a newer browse superseded this one (no duplicate rows or counts)
    tb.replaceChildren(...rows); all = acc;
    // the page's default Headstash (loader HEADSTASH or ?hs=) is listed first
    const dr = defaultHs && [...tb.rows].find((r) => r.querySelector(`button[data-addr="${defaultHs}"]`));
    if (dr) { tb.prepend(dr); dr.dataset.default = "1"; dr.cells[0].append(el("br"), el("span", { className: "badge", textContent: "default" })); }
    $("inst-list").dataset.count = n; applyFilters(); log(`listed ${n} instance(s) for code ${codes.join(",")}`);
  });
}
// ---- fix3: filters (token from bank balances + configured denom; creator from contract info), combined ----
function applyFilters() {
  const keep = new Set(V.filterInstances(all, { tokens: [...filt.tokens], creators: [...filt.creators] }).map((i) => i.addr));
  for (const i of all) i.row.hidden = !keep.has(i.addr);
  $("inst-list").dataset.shown = keep.size;
  const lbl = (k, set) => set.size ? `${k}: ${[...set].map((v) => (k === "Token" ? V.prettyDenom(v) : V.shortAddr(v))).join(", ")}` : `${k}: all`;
  $("f-token").textContent = lbl("Token", filt.tokens); $("f-creator").textContent = lbl("Creator", filt.creators);
  $("f-clear").hidden = !filt.tokens.size && !filt.creators.size;
  $("f-count").textContent = all.length ? `${keep.size} of ${all.length} shown` : "";
}
let fkind = "tokens";
function filterRender() {
  const set = filt[fkind], opts = V.searchOptions(V.filterOptions(all, fkind), $("filter-q").value);
  $("filter-list").replaceChildren(...(opts.length ? opts.map((o) => {
    const cb = el("input", { type: "checkbox", checked: set.has(o.value), value: o.value });
    cb.onchange = () => { cb.checked ? set.add(o.value) : set.delete(o.value); applyFilters(); };
    return el("label", { className: "fopt", title: o.value }, cb, el("span", { textContent: o.label }), el("small", { className: "muted", textContent: ` ${o.value === o.label ? "" : o.value + " · "}${o.n} instance${o.n === 1 ? "" : "s"}` }));
  }) : [el("p", { className: "muted", textContent: "No matches." })]));
}
function filterOpen(kind) { fkind = kind; $("filter-title").textContent = kind === "tokens" ? "Filter by token" : "Filter by creator"; $("filter-q").value = ""; filterRender(); $("filter-dlg").showModal(); $("filter-q").focus(); }
// ---- fix3: footer Details modal (on-chain page + contract details) ----
let feManifest = null, feConfig = null;
function detailsRender() {
  const kv = (k, v) => [el("dt", { textContent: k }), el("dd", {}, v instanceof Node ? v : String(v ?? "–"))];
  const page = [...kv("Frontend contract", feConfig ? copyChip(feConfig.frontend) : "–"), ...kv("Manifest version", feManifest ? feManifest.version : "–"), ...kv("Chain", `${cfg.chain_id} via ${cfg.rest}`), ...kv("Register relay", cfg.relay)];
  const files = feManifest && (feManifest.files || feManifest.chunks) ? Object.entries(feManifest.files || feManifest.chunks).map(([k, f]) => {
    const name = f && f.path ? f.path : (Array.isArray(feManifest.files) ? (f.name || f.path || k) : k), sha = f && (f.sha256 || f.hash || f.digest) || (typeof f === "string" ? f : "");
    return el("li", {}, el("code", { textContent: name }), " ", el("small", { className: "mono", textContent: sha ? "sha256 " + sha : "" }));
  }) : [];
  const box = [el("h3", { textContent: "This page" }), el("dl", {}, ...page), el("p", { className: "muted", textContent: "Every chunk below was checked against its sha256 pin before this page ran." }), el("ul", { className: "pins" }, ...files)];
  if (inst) {
    const ci = inst.ci || {}, h = inst.hal;
    box.push(el("h3", { textContent: "Open Headstash" }), el("dl", {}, ...kv("Label", ci.label || "–"), ...kv("Contract", copyChip(inst.addr)), ...kv("Code id", ci.code_id), ...kv("Creator", ci.creator ? copyChip(ci.creator) : "–"), ...kv("Admin", ci.admin ? copyChip(ci.admin) : "none"),
      ...kv("HAL URI", h ? h.uri : "not set"), ...kv("HAL commitment", h ? `sha256 ${h.sha256 || "–"}  cid ${h.cid || "–"}` : "–"), ...kv("Eligibility root", inst.root)), el("h4", { textContent: "Configuration" }), paramChips({ ...inst, ...ci, tokens: inst.tokens, rt: priv && anc ? { count: anc.count, anchor_count: anc.accepted ? 1 : 0 } : null }, inst.root));
  } else box.push(el("p", { className: "muted", textContent: "Open a Headstash to see its contract details here." }));
  $("details-body").replaceChildren(...box);
}
function detailsOpen() { detailsRender(); $("details-dlg").showModal(); }
// ---- fix3 P0: registration status comes from the chain, never only from the file ----
async function chainSync() {
  if (!state || !inst || state.contract !== inst.addr) return 0;
  const nl = await smart({ note_list: { identity: state.identity } }).then((x) => x ?? null).catch((e) => { log("note_list query failed: " + e.message); return undefined; });
  if (nl === undefined) return 0;
  const ch = V.syncRegistered(state.notes, nl); state.chain_next = nl ? Number(nl.next) : 0;
  // claimed status from the chain too: genesis root is root_id 0, keys are "{root_id}:{nullifier hex}"
  let sp = 0;
  for (const n of state.notes) {
    if (n.st === "spent" || !n.n) continue;
    const hit = await smart({ nullifer: { null: `0:${n.n}` } }).catch(() => false);
    if (hit === true) { n.st = "spent"; sp++; }
  }
  if (sp) { log(`chain: ${sp} note(s) already claimed (nullifier spent); status taken from the chain`); markDirty("claimed"); }
  if (ch) { log(`chain: ${ch} note(s) already registered for ${state.identity.slice(0, 8)}… (note_list next=${nl.next}); status taken from the chain`); markDirty("registered"); }
  return ch + sp;
}
// ---- fix3: per-instance teardown so switching needs no reload ----
function clearNotes() {
  state = null; saved = false; dirty = null; destOk = false; rewardW = null; expanded.clear();
  for (const id of ["tbl", "claim-pick-list", "claim-chain"]) $(id).replaceChildren();
  for (const id of ["summary", "claim-out", "claim-pick-sum", "reg-err", "claim-err", "dst-err", "reward-err", "claim-gate"]) $(id).textContent = "";
  $("nf").value = ""; $("register").disabled = true; rewardLock(); saveUi(); rewardChange();
}
function teardown() {
  if (instScope) instScope.dispose(); instScope = new V.Scope();
  clearNotes(); elig = null; inst = null; priv = null; anc = null; chash = "";
  for (const id of ["anchor-reason", "reg-privacy", "sheet-creator", "sheet-crit"]) $(id).replaceChildren();
  for (const id of ["elig-out", "hal-src", "hal-err", "elig-err"]) $(id).textContent = "";
  $("elig-out").dataset.notes = ""; $("derive").disabled = true; setHal("idle", "Not loaded."); $("sheet-label").textContent = "Headstash";
  anchorPoll();
}
// ---- fix3: MetaMask connect / disconnect / account + chain change ----
function mmUi() {
  const on = !!(who && who.kind === "eip1193");
  $("mm-disconnect").hidden = !on; $("mm-connect").textContent = on ? "Switch account" : "Connect MetaMask";
  $("mm-account").textContent = on ? `Connected: 0x${who.id}` : who ? `Using pasted key 0x${who.id}` : "Not connected";
}
function walletBind() {
  const eth = window.ethereum; if (!eth || walletScope || typeof eth.on !== "function") return;
  walletScope = new V.Scope(); walletScope.on(eth, "accountsChanged", onAccounts); walletScope.on(eth, "chainChanged", onChain);
}
async function onAccounts(acc) {
  return guarded("elig-err", async () => {
    const id = acc && acc[0] ? String(acc[0]).toLowerCase().replace(/^0x/, "") : null;
    if (!id) return mmLocalDisconnect("MetaMask no longer shares an account with this page.");
    if (who && who.id === id) return;
    log(`MetaMask account changed to 0x${id}`);
    if (state && state.identity !== id) {
      if (!saved) { who = { kind: "eip1193", id, addr: "0x" + id }; mmUi(); $("elig-out").textContent = `MetaMask switched to 0x${id}, but the notes derived for 0x${state.identity} are not saved yet. Save them (step 3), then switch again.`; return; }
      clearNotes();
    }
    who = { kind: "eip1193", id, addr: "0x" + id }; mmUi(); if (elig) showElig();
  });
}
function onChain(c) { log(`MetaMask chain changed to ${c} (notes do not depend on the EVM chain); re-reading the account`); if (window.ethereum) window.ethereum.request({ method: "eth_accounts" }).then(onAccounts).catch(() => {}); }
function mmLocalDisconnect(msg) {
  who = null; if (state && saved) clearNotes(); else if (state) msg += " Your derived notes were kept because they are not saved yet.";
  $("elig-out").textContent = msg; $("elig-out").dataset.notes = ""; $("derive").disabled = true; mmUi();
}
async function mmDisconnect() {
  return guarded("elig-err", async () => {
    let revoked = false;
    try { if (window.ethereum) { await window.ethereum.request({ method: "wallet_revokePermissions", params: [{ eth_accounts: {} }] }); revoked = true; } }
    catch (e) { log("wallet_revokePermissions not supported: " + (e.message || e) + "; disconnecting locally"); }
    mmLocalDisconnect(revoked ? "Disconnected: MetaMask permission revoked." : "Disconnected on this page (this wallet cannot revoke permissions; remove the site in MetaMask to fully disconnect).");
  });
}
async function selectInstance(a) {
  return guarded("inst-err", async () => {
    if (state && state.contract !== a && !saved) throw new Error("save the notes for the open Headstash first (step 3), then switch");
    if ((inst && inst.addr !== a) || (state && state.contract !== a)) teardown();
    cfg.headstash = a; $("hs-cur").replaceChildren(copyChip(a)); $("sheet-hs").replaceChildren(copyChip(a)); elig = null; inst = null; priv = null;
    const [camp, hal, ch, dc, th] = await Promise.all([smart({ epoch_campaign: {} }), smart({ hal_source: {} }).catch(() => null), smart({ forward_channels: {} }).catch(() => []), smart({ direct_claim: {} }).catch(() => null), smart({ theme: {} }).catch(() => null)]);
    const ci = (await lcd(`/cosmwasm/wasm/v1/contract/${a}`).catch(() => ({}))).contract_info || {};
    const bal = await lcd(`/cosmos/bank/v1beta1/balances/${a}`).catch(() => ({ balances: [] }));
    chash = hex(await sha256(te.encode(a)));
    $("sheet-label").textContent = ci.label || "Headstash";
    $("sheet-creator").replaceChildren(...(ci.creator ? ["creator ", copyChip(ci.creator)] : []));
    themeCands.headstash = th ?? null; themeApply();
    const ctx = hex(await directCtx());
    const exp = dc && (dc.expected_ctx || dc.ctx); if (exp && rootHex(exp) !== ctx) throw new Error("direct-claim ctx mismatch: wrong chain id or contract");
    inst = { addr: a, camp, hal, ci, tokens: V.instanceTokens(bal.balances, cfg.denom), channels: listOf(ch, "channels").filter((c) => c.enabled), root: rootHex(camp?.registration?.eligibility_root) };
    $("sheet-crit").replaceChildren(critChips(inst));
    if (!inst.root) throw new Error("instance has no registration eligibility root");
    $("hal-src").textContent = hal ? `${hal.uri}  sha256=${hal.sha256 || "–"}  cid=${hal.cid || "–"}` : "not set";
    $("fwd-channel").replaceChildren(...inst.channels.map((c) => el("option", { value: c.channel_id, textContent: `${c.channel_id} (timeout ${c.timeout_seconds}s)` })));
    fwdState();
    log(`selected ${a}; root ${inst.root.slice(0, 16)}…; ${inst.channels.length} forward channel(s)`);
    await privacyRefresh();
    if (hal) await halFetch();
    return true;
  });
}
// fix2: the forward option is only offered when the contract allowlists an enabled IBC channel; otherwise say why, inline.
function fwdState() {
  const r = document.querySelector("input[name=dest-mode][value=forward]"), n = inst ? inst.channels.length : 0;
  r.disabled = !n; $("fwd-reason").hidden = !!n; $("fwd-reason").dataset.reason = !inst ? "loading" : n ? "" : "no-channel";
  $("fwd-reason").textContent = !inst ? "Checking this Headstash for allowlisted forward channels…" : n ? "" : "Forward to Penumbra is unavailable for this Headstash: its contract allowlists no IBC forward channel (forward_channels is empty). Claims go to a fresh ephemeral account instead.";
  if (r.disabled && r.checked) { document.querySelector("input[name=dest-mode][value=ephemeral]").checked = true; $("fwd-box").hidden = true; }
}
// ---- R13: bottom sheet (native modal <dialog>: aria-modal, focus containment, Esc) ----
async function openInstance(a, btn) {
  opener = btn || null;
  if (cfg.headstash !== a || !inst) { if (!(await selectInstance(a))) return; }
  openSheet();
}
function openSheet() {
  const d = $("hs-sheet"); if (!d.open) d.showModal(); $("sheet-resume").hidden = false; flow();
  const f = cur ? $(`sec-${cur}-toggle`) : $("sheet-close"); f.focus();
}
const SECS = ["eligibility", "register", "save", "wallet", "claim"];
const HINT = { eligibility: "Select a Headstash first.", register: "Needs an eligible address (1) or a loaded notes file.", save: "Needs derived notes (2).",
  wallet: "Needs registered notes (2).", claim: "Needs registered notes and a confirmed destination (4)." };
function progress() {
  const notes = state ? state.notes : [], eligible = !!(who && elig && (elig.get(who.id) || []).length);
  const reg = notes.some((n) => n.st !== "new"), allReg = notes.length > 0 && notes.every((n) => n.st !== "new");
  // [unlocked, done] — each component unlocks only once its prerequisite is met
  return { eligibility: [!!inst, eligible || !!state], register: [eligible || !!state, allReg], save: [!!state, saved], wallet: [reg, destOk],
    claim: [reg && destOk, notes.length > 0 && notes.every((n) => n.st === "spent")] };
}
function summaryOf(s) {
  if (s === "eligibility") return $("elig-out").textContent || (state ? `notes file loaded (${state.notes.length} notes)` : "");
  if (s === "register") return $("summary").textContent;
  if (s === "save") return saved ? "Notes file saved." : "";
  if (s === "wallet") return destMode() === "forward" ? `Forward to Penumbra via ${$("fwd-channel").value || "?"}` : "Fresh ephemeral account per note";
  return $("claim-out").textContent;
}
function flow() {
  if (!cfg || !$("sec-claim")) return; const p = progress();
  const next = SECS.find((s) => p[s][0] && !p[s][1]) || null;
  if (next !== cur) { cur = next; expanded.clear(); }
  for (const s of SECS) {
    const [un, done] = p[s], st = !un ? "locked" : s === cur ? "current" : done ? "done" : "open", show = un && (s === cur || expanded.has(s));
    const tg = $(`sec-${s}-toggle`); $(`sec-${s}`).dataset.status = st; $(`sec-${s}-body`).hidden = !show; tg.disabled = !un; tg.setAttribute("aria-expanded", String(show));
    $(`sec-${s}-badge`).textContent = { locked: "locked", current: "next step", done: "done", open: "" }[st];
    $(`sec-${s}-hint`).textContent = !un ? HINT[s] : !show ? summaryOf(s) : "";
  }
  const claimable = !!state && state.notes.some((n) => n.st === "registered" || n.st === "submitted");
  const gate = V.claimGate(state ? state.notes : [], anc); $("claim-gate").textContent = state ? gate.reason : ""; $("claim-gate").dataset.ok = String(gate.ok);
  $("claim").disabled = !(claimable && gate.ok && V.privacyGateOpen(priv, $("privacy-ack").checked));
}
// ---- R13: privacy set = registration_tree.count − direct_claim.claimed (public contract state) ----
async function privacyRefresh() {
  if (!inst && !cfg.headstash) return;
  if (state && inst) { if (await chainSync()) render(); }
  try {
    const [rt, dc, blk] = await Promise.all([smart({ registration_tree: {} }), smart({ direct_claim: {} }), lcd("/cosmos/base/tendermint/v1beta1/blocks/latest").catch(() => null)]); priv = V.privacySet(rt, dc);
    const ht = blk && blk.block && blk.block.header ? blk.block.header.height : null;
    anc = V.anchorStatus(rt, ht, inst && inst.camp && inst.camp.registration ? inst.camp.registration.anchor_interval : null); anc.checked = new Date();
  }
  catch (e) { priv = null; anc = null; log("privacy set unavailable: " + e.message); }
  privacyShow();
}
let anc = null, ancTimer = 0;
const ANCHOR_POLL_S = 15;
function anchorShow() {
  const box = $("anchor-reason"), s = anc;
  $("claim-chain").replaceChildren(...V.claimChain(state ? state.notes : [], s).map((x) => { const c = chip("", x.label); c.dataset.step = x.id; c.dataset.state = x.state;
    if (x.state === "current") c.setAttribute("aria-current", "step"); c.title = { done: "done", current: "current step", todo: "waiting" }[x.state]; return el("li", {}, c); }));
  if (!s) { box.replaceChildren(); box.dataset.state = "unknown"; return; }
  box.dataset.state = s.state; Object.assign(box.dataset, { count: s.count, min: s.min, next: s.next, height: s.height ?? "" });
  const nums = el("div", { className: "chips" }, chip("registered", s.count), chip("min anonymity set", s.min), chip("anchor covers", s.accepted ? s.leaves : "none"), chip("next anchor ≥ block", s.next), chip("now block", s.height ?? "?"));
  if (s.interval) nums.append(chip("anchor interval", `${s.interval} block${s.interval === 1 ? "" : "s"}`));
  const chk = s.checked ? s.checked.toLocaleTimeString() : "";
  box.replaceChildren(el("b", { textContent: s.reason }), nums, el("p", { className: "muted", textContent: `${s.eta ? s.eta + " " : ""}Checked ${chk}; this page re-checks every ${ANCHOR_POLL_S} s while the sheet is open.` }));
}
function anchorPoll() { if (!instScope) instScope = new V.Scope(); ancTimer = instScope.interval(() => { if ($("hs-sheet").open && inst && (!anc || anc.state !== "accepted")) privacyRefresh(); }, ANCHOR_POLL_S * 1000); }
function privacyShow() {
  const p = priv, box = $("privacy");
  $("privacy-reg").textContent = p ? p.registered : "–"; $("privacy-claimed").textContent = p ? p.claimed : "–"; $("privacy-set").textContent = p ? p.set : "–"; $("privacy-min").textContent = p ? p.threshold : "–";
  if (p) Object.assign(box.dataset, { registered: p.registered, claimed: p.claimed, set: p.set, threshold: p.threshold, anchored: p.anchored });
  box.dataset.small = p ? String(p.small) : "unknown";
  $("privacy-meter").value = p ? Math.min(1, p.set / p.threshold) : 0;
  $("privacy-advice").textContent = p ? V.privacyAdvice(p) : "Could not read the privacy set from the chain. Confirm below to claim anyway.";
  $("privacy-ack-l").hidden = !!p && !p.small;
  const rp = $("reg-privacy");
  if (p && state && state.notes.some((n) => n.st !== "new")) { rp.replaceChildren(el("span", { textContent: "Privacy set now" }), chip("set", p.set), el("span", { textContent: "=" }), chip("registered", p.registered), el("span", { textContent: "−" }), chip("claimed", p.claimed), el("span", { textContent: "· suggested minimum before claiming" }), chip("min", p.threshold)); Object.assign(rp.dataset, { set: p.set, registered: p.registered, claimed: p.claimed, threshold: p.threshold }); }
  else rp.replaceChildren();
  anchorShow();
  // EXTENSION POINT (R13): a delayed-claim relay would be listed here; this build has none (V.DELAYED_CLAIM_RELAYS is empty).
  const rel = V.delayedRelays(); $("delay-relay").hidden = !rel.length; $("delay-relay").dataset.count = rel.length;
  flow();
}

// ---- R1: HAL fetch + verify ----
function setHal(st, t) { $("hal-status").dataset.state = st; $("hal-status").textContent = t; }
async function useHal(bytes, origin) {
  const v = await V.verifyHal(bytes, inst.hal), p = V.parseHal(bytes, inst.root);
  elig = p.byIdentity; setHal("ok", `Verified (${v.passed.join(" + ")}; sha256 ${v.sha256.slice(0, 16)}…) from ${origin}: ${p.hal.rows.length} notes for ${elig.size} identities, root matches chain.`);
  $("hal-status").dataset.sha = v.sha256; log("HAL verified", v.sha256); if (who && !state) showElig();
}
async function halFetch() {
  return guarded("hal-err", async () => {
    if (!inst) throw new Error("select an instance first"); if (!inst.hal) throw new Error("this instance has no HAL source; ask the operator to set_hal_source");
    elig = null; setHal("busy", "Fetching…"); const errs = [];
    for (const u of V.resolveHal(inst.hal)) {
      try { const r = await fetch(u, { credentials: "omit", referrerPolicy: "no-referrer" }); if (!r.ok) throw new Error("HTTP " + r.status); return await useHal(new Uint8Array(await r.arrayBuffer()), new URL(u).host); }
      catch (e) { errs.push(`${u}: ${e.message}`); if (/mismatch|refus|format|another eligibility/.test(e.message)) break; }
    }
    setHal("error", "Verification failed: page will not use this data."); throw new Error(errs.join("; "));
  });
}
async function halLocal() {
  return guarded("hal-err", async () => {
    if (!inst || !inst.hal) throw new Error("select an instance with a HAL source first");
    elig = null; const f = $("hal-file").files[0]; if (!f) return;
    try { await useHal(new Uint8Array(await f.arrayBuffer()), "local file"); } catch (e) { setHal("error", "Verification failed: page will not use this data."); throw e; }
  });
}

// ---- R2: eligibility via MetaMask (address only; lookup is local) ----
function showElig() {
  const rows = elig.get(who.id) || [];
  const tot = rows.reduce((a, r) => a + (r.amount != null ? Number(r.amount) : 0), 0);
  $("elig-out").textContent = rows.length ? `0x${who.id} is eligible: ${rows.length} notes${tot ? `, ${tot} ${cfg.denom}` : ""}.` : `0x${who.id} is not in this eligibility set.`;
  $("elig-out").dataset.notes = rows.length; $("derive").disabled = !rows.length || !!state;
}
async function mmConnect() {
  return guarded("elig-err", async () => {
    if (!elig) throw new Error("verify the HAL first (step 2)");
    const eth = window.ethereum; if (!eth) throw new Error("no MetaMask (EIP-1193) provider found");
    const acc = await eth.request({ method: "eth_requestAccounts" }); if (!acc || !acc[0]) throw new Error("no account shared");
    const id = acc[0].toLowerCase().replace(/^0x/, ""); if (!/^[0-9a-f]{40}$/.test(id)) throw new Error("bad account from wallet");
    if (state && state.identity !== id && saved) clearNotes();
    who = { kind: "eip1193", id, addr: "0x" + id }; walletBind(); mmUi(); showElig();
  });
}
async function keyUse() {
  return guarded("elig-err", async () => {
    if (!elig) throw new Error("verify the HAL first (step 2)");
    const k = unhex($("key").value.trim()); $("key").value = ""; if (!C.privOk(k)) { k.fill(0); throw new Error("invalid secp256k1 private key"); }
    if (rawKey) rawKey.fill(0); rawKey = k; who = { kind: "raw", id: hex(C.keccak256(C.pubkey(k).uncompressed).slice(12)) }; showElig();
  });
}
const sigBytes = (h) => { const s = unhex(h.replace(/^0x/, "")); if (s.length !== 65) throw new Error("wallet returned a bad signature"); s[64] = 27 + ((s[64] >= 27 ? s[64] - 27 : s[64]) & 1); return s; };
async function personalSign(msg) { return sigBytes(await window.ethereum.request({ method: "personal_sign", params: ["0x" + hex(te.encode(msg)), who.addr] })); }

// ---- R3: reward address ----
async function rewardAddr() {
  const mode = document.querySelector("input[name=reward-mode]:checked").value;
  if (mode === "keplr") {
    const k = window.keplr; if (!k) throw new Error("Keplr not found");
    await k.enable(cfg.chain_id); const key = await k.getKey(cfg.chain_id); rewardW = null;
    return checkTerp(key.bech32Address);
  }
  if (!rewardW) rewardW = await wallet(C.reduceN(crypto.getRandomValues(new Uint8Array(32))));
  return rewardW.addr;
}
// fix2: once notes exist the reward address is bound into the registration signature: the radios are disabled and labelled, never an error.
function rewardLock() { const l = !!state; document.querySelectorAll("input[name=reward-mode]").forEach((r) => (r.disabled = l)); $("reward-lock").hidden = !l; $("reward-addr").dataset.locked = String(l); }
async function rewardChange() { if (state) return rewardLock(); return guarded("reward-err", async () => { $("reward-addr").textContent = await rewardAddr(); }); }

// ---- derivation ----
async function directCtx() { const h = await sha256(te.encode(`terp-hs-direct-ctx-v1\0${cfg.chain_id}\0${cfg.headstash}`)); h[31] &= 0x3f; return h; }
async function listHash(ctx, tag, ident, notes) {
  const parts = [te.encode("terp-hs-note-list-v2"), ctx, new Uint8Array([tag]), ident, C.u16le(notes.length)];
  notes.forEach((n, i) => parts.push(new Uint8Array([n.cls]), C.u16le(i), unhex(n.k)));
  return sha256(cat(...parts));
}
const hk = async (s, info, extra = new Uint8Array()) => C.reduceN(await C.hkdf(unhex(s), te.encode(info), extra, 32));
async function claimWallet(s) { return wallet(await hk(s, "terp-hs-claim-key-v1")); }
async function fallbackWallet(s, attempt) { return wallet(await hk(s, "terp-hs-fwd-fallback-v1", C.u32le(attempt))); }
function checkTerp(a) { const d = C.bech32Decode(a); if (d.hrp !== cfg.hrp || (d.bytes.length !== 20 && d.bytes.length !== 32)) throw new Error("not a " + cfg.hrp + " address: " + a); return a; }
const seedMessage = (ctx) => `terp-headstash-note-seed-v2\nchain: ${cfg.chain_id}\ncontract: ${cfg.headstash}\nctx: ${ctx}\n\nSigning derives your private Headstash notes. Only sign this on the Headstash claim page.`;

async function derive() {
  return guarded("reg-err", async () => {
    if (!who || !elig) throw new Error("check eligibility first"); if (state) throw new Error("notes already derived");
    const rows = elig.get(who.id) || []; if (!rows.length) throw new Error("not eligible");
    const reward = await rewardAddr(); $("reward-addr").textContent = reward;
    const ctx = await directCtx(), ch = hex(ctx), ident = unhex(who.id);
    let ikm, info;
    if (who.kind === "eip1193") {
      log("asking the wallet to sign the note seed (twice, to confirm it is deterministic)");
      const a = await personalSign(seedMessage(ch)), b = await personalSign(seedMessage(ch));
      if (hex(a) !== hex(b)) throw new Error("this wallet's signatures are not deterministic, so notes could not be re-derived: use the advanced raw-key option");
      ikm = await sha256(a.slice(0, 64)); info = "terp-hs-note-secret-v2";
    } else { if (!rawKey) throw new Error("enter the key again"); ikm = rawKey; info = "terp-hs-note-secret-v1"; }
    const notes = [];
    for (const r of rows) {
      const wide = await C.hkdf(ikm, te.encode(info), cat(ctx, ident, C.u32le(r.note_index)), 64);
      const s = Z.note_secret_from_wide_hex(hex(wide)); wide.fill(0);
      const n = { c: ch, i: r.note_index, li: r.leaf_index, p: r.path, cls: r.class, amt: r.amount != null ? String(r.amount) : null, s, k: Z.membership_key_com_hex(s), n: Z.note_nullifier_hex(s, ch), st: "new" };
      n.dest = (await claimWallet(s)).addr; notes.push(n);
    }
    const lh = await listHash(ctx, 1, ident, notes);
    const msg = `headstash-register-notes-v3:${ch}:${hex(lh)}:${notes.length}:${reward}`;
    let sig;
    if (who.kind === "eip1193") sig = await personalSign(msg);
    else { const m = te.encode(msg), s = await C.sign(rawKey, C.keccak256(cat(te.encode("\x19Ethereum Signed Message:\n" + m.length), m))); sig = cat(s.r, s.s, new Uint8Array([27 + (s.rec & 1)])); }
    ikm.fill(0); if (rawKey) { rawKey.fill(0); rawKey = null; }
    state = { v: 1, ui: 2, derivation: who.kind === "eip1193" ? "eip1193-seed-v2" : "raw-key-v1", chain_id: cfg.chain_id, contract: cfg.headstash, identity: who.id,
      created: Math.floor(Date.now() / 1000), reward_addr: reward, reward_priv: rewardW ? hex(rewardW.priv) : undefined,
      registration: { list_hash: hex(lh), signature: hex(sig) }, notes };
    log(`derived ${notes.length} notes; list_hash ${hex(lh).slice(0, 16)}…`); render(); await save();
  });
}

// ---- notes file ----
const canon = (o) => Array.isArray(o) ? "[" + o.map(canon).join(",") + "]" : o && typeof o === "object" ? "{" + Object.keys(o).sort().filter((k) => o[k] !== undefined).map((k) => JSON.stringify(k) + ":" + canon(o[k])).join(",") + "}" : JSON.stringify(o);
async function fileBytes() {
  const o = JSON.parse(JSON.stringify(state)); delete o.sha256; o.sha256 = hex(await sha256(te.encode(canon(o))));
  let out = JSON.stringify(o, null, 1); const pass = $("pass").value;
  if (pass) {
    const salt = crypto.getRandomValues(new Uint8Array(16)), iv = crypto.getRandomValues(new Uint8Array(12));
    const k = await crypto.subtle.deriveKey({ name: "PBKDF2", hash: "SHA-256", salt, iterations: 600000 }, await crypto.subtle.importKey("raw", te.encode(pass), "PBKDF2", false, ["deriveKey"]), { name: "AES-GCM", length: 256 }, false, ["encrypt"]);
    out = JSON.stringify({ v: 1, enc: "pbkdf2-aesgcm", salt: b64(salt), iv: b64(iv), ct: b64(new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, k, te.encode(out)))) });
  }
  return out;
}
function download(name, text) { const a = el("a", { href: URL.createObjectURL(new Blob([text], { type: "application/json" })), download: name }); document.body.appendChild(a); a.click(); a.remove(); }
// fix2: the notes file downloads once (at derive). Later status changes (registered / claimed) are offered as an explicit,
// differently named re-save; the first file stays valid (re-registering or re-claiming a note is recognised and skipped).
let dirty = null; const SAVE_LABEL = "Save notes file";
function saveName() { return V.notesFileName(chash, state && state.identity, dirty); }
function saveUi() {
  $("save").textContent = dirty ? `Save updated notes (now marked ${dirty})` : SAVE_LABEL; $("save").dataset.dirty = dirty || "";
  $("save-note").textContent = dirty ? `Optional: your notes are now marked ${dirty}. The file you already saved still works for claiming; saving again writes ${saveName()} so it does not overwrite or duplicate the first file.` : "";
}
async function save() { const name = saveName(); download(name, await fileBytes()); saved = true; dirty = null; log(`notes file saved as ${name}`); saveUi(); flow(); }
function markDirty(what) { dirty = what; saveUi(); log(`notes now marked ${what}: use "Save updated notes" in step 3 to keep a copy with this status`); }
async function load() {
  return guarded("reg-err", async () => {
    const f = $("nf").files[0]; if (!f) throw new Error("choose a notes file");
    let o = JSON.parse(await f.text());
    if (o.enc === "pbkdf2-aesgcm") {
      const k = await crypto.subtle.deriveKey({ name: "PBKDF2", hash: "SHA-256", salt: unb64(o.salt), iterations: 600000 }, await crypto.subtle.importKey("raw", te.encode($("pass-load").value || $("pass").value), "PBKDF2", false, ["deriveKey"]), { name: "AES-GCM", length: 256 }, false, ["decrypt"]);
      try { o = JSON.parse(new TextDecoder().decode(await crypto.subtle.decrypt({ name: "AES-GCM", iv: unb64(o.iv) }, k, unb64(o.ct)))); } catch { throw new Error("wrong passphrase"); }
      if ($("pass-load").value) { $("pass").value = $("pass-load").value; $("pass-load").value = ""; } // re-saves stay encrypted with the same passphrase
    }
    const want = o.sha256; delete o.sha256;
    if (o.v !== 1 || hex(await sha256(te.encode(canon(o)))) !== want) throw new Error("notes file checksum mismatch or unknown version");
    if (o.chain_id !== cfg.chain_id) throw new Error("notes file is for another chain");
    V.checkNotesContract(o, inst ? inst.addr : null);
    state = o; if (o.reward_priv) rewardW = await wallet(unhex(o.reward_priv)); $("reward-addr").textContent = o.reward_addr;
    saved = true; log(`loaded ${o.notes.length} notes (checksum ok)`); await chainSync(); render(); await privacyRefresh();
  });
}

// ---- register ----
async function register() {
  return guarded("reg-err", async () => {
    await chainSync(); render();
    const all = state.notes; if (!all.some((n) => n.st === "new")) { $("summary").dataset.chain = "registered"; return log("nothing to register: the chain already has every note registered"); }
    const list = all.map((n) => ({ class: n.cls, key_com: b64(unhex(n.k)) }));
    for (let start = 0; start < all.length; start += 32) {
      const chunk = all.slice(start, start + 32); if (chunk.every((n) => n.st !== "new")) continue;
      const first = start === 0;
      const body = { signer: { evm: { address: b64(unhex(state.identity)) } }, signature: first ? b64(unhex(state.registration.signature)) : null, list, start,
        eligibility: chunk.map((n) => ({ index: n.li, path: n.p.map((x) => b64(unhex(x))) })), reward_addr: first ? state.reward_addr : null };
      const w = await wallet(C.reduceN(crypto.getRandomValues(new Uint8Array(32))));
      const g = await grant(w.addr, "register", body).catch(async (e) => { if (/already registered/i.test(e.message)) { await chainSync(); return null; } throw e; });
      if (!g) { render(); continue; }
      const res = await execTx(w, { register_notes: body }, cfg.register_gas, g.granter);
      if (res.code === 0 || /already registered/i.test(res.raw_log || "")) { chunk.forEach((n) => { n.st = "registered"; n.rtx = res.txhash; }); log(`registered notes ${start}..${start + chunk.length - 1} tx ${res.txhash}`); }
      else throw new Error(`register chunk ${start} failed code ${res.code}: ${(res.raw_log || "").slice(0, 200)}`);
      render();
    }
    log("registered all notes"); markDirty("registered"); render(); await privacyRefresh();
  });
}
async function anchorCheck() {
  return guarded("claim-err", async () => {
    const rt = await smart({ registration_tree: {} }); await privacyRefresh();
    $("anchor-out").textContent = rt.latest_anchor ? `Latest anchor ${hex(unb64(rt.latest_anchor.root)).slice(0, 16)}… covering ${rt.latest_anchor.leaf_count} registrations (tree size ${rt.count}, minimum anonymity set ${rt.min_anonymity_set}).` : "No accepted anchor yet: claims open after the next anchor.";
  });
}

// ---- R4/R5: destination ----
const destMode = () => document.querySelector("input[name=dest-mode]:checked").value;
// Zafu (Penumbra) via the documented web-extension path: window[Symbol.for("penumbra")][origin].connect() -> MessagePort ->
// @penumbra-zone/transport-dom channel transport -> generated connect-rpc ViewService client (zafu-client.js, bundled on chain).
// Each forward claim asks for AddressByIndex at a FRESH {account, randomizer} and proves it with IndexByAddress (docs/V2-SPEC.md §Zafu).
let ZC = null, zafu = null, book = null;
const zafuSet = (state, msg) => { $("zafu-detect").textContent = msg; $("zafu-detect").dataset.state = state; $("zafu-connect").hidden = state === "connected"; $("zafu-disconnect").hidden = state !== "connected"; };
async function zafuLib() { if (!ZC) ZC = await import(cfg.urls["zafu-client.js"]); return ZC; }
// No extension-ID pin or allowlist (user-built / modified wallets are fine): any provider announced under
// window[Symbol.for("penumbra")] is listed with its announced origin + manifest and the user picks. Safety comes from
// checking every returned address (bech32m + IndexByAddress ownership + IndexBook no-reuse), not from the wallet's identity.
let wallets = [];
const walletLabel = (w) => `${w.origin}${w.manifest ? " (manifest " + w.manifest.slice(0, 120) + ")" : ""}`;
async function zafuDetect() {
  const L = await zafuLib(), ps = L.providers(window);
  if (!ps.length) return zafuSet("missing", "No Penumbra wallet detected. Install Zafu or another Penumbra wallet (or paste a one-time address below)."), null;
  if (ps.length !== wallets.length || ps.some((p, i) => p.origin !== wallets[i].origin)) {
    const prev = wallets[Number($("zafu-wallet").value)]?.origin;
    wallets = ps.map((p) => ({ ...p, manifest: typeof p.provider.manifest === "string" ? p.provider.manifest : "" }));
    const sel = $("zafu-wallet"); sel.replaceChildren(...wallets.map((w, i) => el("option", { value: String(i), textContent: walletLabel(w) })));
    sel.value = String(Math.max(0, wallets.findIndex((w) => w.origin === prev))); sel.hidden = $("zafu-wallet-l").hidden = wallets.length < 2;
    sel.dataset.count = wallets.length;
  }
  const pick = wallets[Number($("zafu-wallet").value)] || wallets[0];
  if (!zafu) zafuSet("found", `${wallets.length > 1 ? wallets.length + " Penumbra wallets announced; selected " : "Penumbra wallet announced: "}${walletLabel(pick)}. Press Connect Zafu.`);
  return pick;
}
async function zafuConnect() {
  const L = await zafuLib(), pick = await zafuDetect(); if (!pick) throw new Error("no Penumbra wallet detected: install Zafu, reload the page, or paste an address");
  zafuSet("pending", "Waiting for approval in Zafu…");
  try {
    await pick.provider.connect(); // approval popup on first use; rejects with cause Denied / NeedsLogin / NotHandled
    const client = L.makeViewClient(pick.provider);
    zafu = { origin: pick.origin, provider: pick.provider, client };
    zafuSet("connected", `Connected to Penumbra wallet ${walletLabel(pick)}. A fresh address is requested for every forward claim.`);
  } catch (e) { zafu = null; zafuSet("error", V.zafuErrorText(e)); throw new Error(V.zafuErrorText(e)); }
}
async function zafuDisconnect() {
  const z = zafu; zafu = null;
  try { if (z && typeof z.provider.disconnect === "function") await z.provider.disconnect(); } catch (e) { log("Zafu disconnect: " + (e.message || e)); }
  zafuSet("found", "Disconnected from Zafu. Connect again, or paste a one-time address.");
}
/** A fresh diversified address for `owner` (note nullifier): new index, wallet-proved, recorded so it is never reused. */
async function zafuAddress(owner) {
  if (!zafu) await zafuConnect();
  const L = await zafuLib(), idx = book.allocate(V.parseAccount($("zafu-account").value));
  let a; try { a = await L.addressAtIndex(zafu.client, idx); } catch (e) { throw new Error(V.zafuErrorText(e)); }
  V.checkPenumbra(a.address, C.bech32Decode); await guard.check(a.address, owner);
  book.record(idx, a.address, owner); $("recv").value = a.address; return a.address;
}
async function receiverFor(first, owner) {
  if (zafu && !$("recv-any").checked) return zafuAddress(owner);
  if (!first) return null;
  return $("recv").value.trim();
}
function checkReceiver(r) {
  if (!V.validReceiver(r)) throw new Error("receiver must be 1-256 printable characters");
  if (!$("recv-any").checked) V.checkPenumbra(r, C.bech32Decode);
  return r;
}

// ---- claim ----
async function claim() {
  return guarded("claim-err", async () => {
    await privacyRefresh();
    if (!V.privacyGateOpen(priv, $("privacy-ack").checked)) throw new Error("the privacy set is below the suggested minimum: wait for it to grow, or tick the confirmation to claim now");
    try { await claimRun(); } finally { await privacyRefresh(); }
  });
}
async function claimRun() {
  {
    const mode = destMode(), pick = new Set([...document.querySelectorAll("input[name=claim-note]:checked")].map((x) => Number(x.value)));
    const rt = await smart({ registration_tree: {} }); if (!rt.latest_anchor) { const s = V.anchorStatus(rt, anc ? anc.height : null); throw new Error(`no accepted anchor yet: ${s.reason}`); }
    const root = rt.latest_anchor.root, rh = hex(unb64(root)), todo = state.notes.filter((x) => (x.st === "registered" || x.st === "submitted") && pick.has(x.i));
    if (!todo.length) throw new Error("select at least one note to claim");
    let ch = null; if (mode === "forward") { ch = $("fwd-channel").value; if (!V.validChannel(ch) || !inst.channels.some((c) => c.channel_id === ch)) throw new Error("pick an allowlisted forward channel"); }
    let done = 0;
    for (const [k, n] of todo.entries()) {
      let pth;
      try { pth = await smart({ registration_path: { key_com: b64(unhex(n.k)), anchor: root } }); }
      catch (e) { log(`note ${n.i}: not in the latest anchor yet (${e.message.slice(0, 80)})`); continue; }
      const w = await claimWallet(n.s); await guard.use(w.addr, "claim", n.n);
      let dest = w.addr, t = null;
      if (mode === "forward") {
        const recv = await receiverFor(k === 0, n.n); if (!recv) { log("forward: each note needs a new one-time address; paste the next one and claim again"); break; }
        checkReceiver(recv); await guard.check(recv, n.n);
        n.fa = (n.fa || 0) + 1; const fb = await fallbackWallet(n.s, n.fa); await guard.use(fb.addr, "fallback", n.n);
        await guard.use(recv, "receiver", n.n);
        t = { channel_id: ch, receiver: recv, memo: null, fallback: fb.addr };
        dest = await V.forwardDestination(cfg.chain_id, cfg.headstash, t);
        const q = await smart({ forward_destination: { forward: t } }); const qd = typeof q === "string" ? q : q && (q.destination || q.forward_destination);
        if (qd !== dest) throw new Error(`forward destination mismatch (page ${dest}, contract ${qd}): refusing`);
        $("fwd-preview").textContent = `note ${n.i}: ${ch} → ${recv.slice(0, 24)}…, fallback ${fb.addr}`;
      }
      log(`note ${n.i}: proving…`); await sleep(30);
      const t0 = performance.now();
      const pr = JSON.parse(Z.prove_note_witness_browser(JSON.stringify({ secret_hex: n.s, ctx_hex: n.c, root_hex: rh, destination: dest, amount: String(pth.amount), index: pth.index, siblings_hex: pth.siblings })));
      const p = pr.body.split(":"); if (p[3] !== n.n) throw new Error("nullifier mismatch between notes file and proof");
      n.prove_ms = Math.round(performance.now() - t0); n.amt = p[5];
      const msg = { root, nullifier: b64(unhex(p[3])), amount: Number(p[5]), proof: b64(unhex(pr.proof_hex)) };
      if (t) msg.forward = t;
      const g = await grant(w.addr, t ? "claim_forward" : "claim", msg);
      n.st = "submitted"; render();
      const res = await execTx(w, t ? { claim_note_forward: msg } : { claim_note: msg }, t ? cfg.forward_gas || 900000 : cfg.claim_gas, g.granter); n.tx = res.txhash;
      if (res.code === 0 || /nullifier.*(spent|used)|already (claimed|spent)/i.test(res.raw_log || "")) {
        n.st = "spent"; n.mode = mode; done++; if (t) n.fwd = { ...t, destination: dest };
        log(`note ${n.i}: ${t ? "forwarded" : "claimed"} ${p[5]} → ${t ? t.receiver.slice(0, 20) + "… via " + ch : dest} (proof ${n.prove_ms} ms) tx ${res.txhash}`);
      } else throw new Error(`note ${n.i}: claim failed code ${res.code}: ${(res.raw_log || "").slice(0, 200)}`);
      render();
    }
    $("claim-out").textContent = `${done} note(s) ${mode === "forward" ? "forwarded" : "claimed"} in this run.`; $("claim-out").dataset.done = done;
    if (done) markDirty("claimed"); log(`claim run finished: ${done} note(s)`);
  }
}
async function exportWallets() {
  const out = [];
  for (const n of state.notes) {
    const w = await claimWallet(n.s), e = { note: n.i, address: w.addr, priv_hex: hex(w.priv) };
    if (n.fa) { e.fallbacks = []; for (let a = 1; a <= n.fa; a++) { const f = await fallbackWallet(n.s, a); e.fallbacks.push({ attempt: a, address: f.addr, priv_hex: hex(f.priv) }); } }
    out.push(e);
  }
  download("headstash-claim-wallets.json", JSON.stringify(out, null, 1));
}
function render() {
  if (!state) return; const by = {}; state.notes.forEach((n) => (by[n.st] = (by[n.st] || 0) + 1));
  $("summary").textContent = `0x${state.identity}: ${state.notes.length} notes — ` + Object.entries(by).map(([k, v]) => `${k} ${v}`).join(", ");
  $("summary").dataset.counts = JSON.stringify(by);
  $("derive").disabled = true; $("register").disabled = !by.new; 
  $("save").disabled = false; $("export").disabled = false; $("reward-export").disabled = !state.reward_priv;
  const head = el("tr", {}, ...["#", "class", "status", "destination"].map((h) => el("th", { textContent: h })));
  pickRender(); rewardLock(); saveUi();
  $("tbl").replaceChildren(head, ...state.notes.map((n) => el("tr", {}, ...[n.i, n.cls, n.st, n.fwd ? `${n.fwd.channel_id} → ${n.fwd.receiver.slice(0, 22)}…` : n.dest].map((x) => el("td", { textContent: String(x) })))));
  flow();
}
const noteAmt = (n) => { const x = inst && inst.camp && inst.camp.amounts ? inst.camp.amounts[n.cls] : null; return x ? x.amount : null; };
function pickRender() {
  const box = $("claim-pick-list"), prev = new Map([...box.querySelectorAll("input[name=claim-note]")].map((x) => [Number(x.value), x.checked]));
  const cl = state.notes.filter((n) => n.st === "registered" || n.st === "submitted");
  box.replaceChildren(...cl.map((n) => { const cb = el("input", { type: "checkbox", name: "claim-note", value: String(n.i), checked: prev.has(n.i) ? prev.get(n.i) : true }); cb.onchange = pickSum;
    const a = noteAmt(n); return el("label", { className: "inl" }, cb, chip("note", `#${n.i}`), a != null ? chip("amount", amt(a)) : chip("class", n.cls)); }));
  if (!cl.length) box.append(el("span", { className: "muted", textContent: state.notes.some((n) => n.st === "new") ? "Register your notes first." : "No unclaimed notes left." }));
  pickSum();
}
function pickSum() {
  const sel = state ? state.notes.filter((n) => [...document.querySelectorAll("input[name=claim-note]:checked")].some((x) => Number(x.value) === n.i)) : [];
  const tot = sel.reduce((s, n) => s + (Number(noteAmt(n)) || 0), 0);
  $("claim-pick-sum").textContent = sel.length ? `${sel.length} note${sel.length === 1 ? "" : "s"} selected${tot ? `, ${amt(tot)} in total` : ""}. Each note is proved and claimed separately.` : "No note selected.";
  $("claim-pick").dataset.selected = sel.length;
}
function info(id) {
  const s = STEPS[id]; if (!s) return; $("info-title").textContent = s.title; $("info-what").textContent = s.what; $("info-reveals").textContent = s.reveals; $("info-risks").textContent = s.risks;
  $("info-dlg").dataset.step = id; $("info-dlg").showModal();
}

// ---- R12: theme (headstash contract > manifest default > built-in), light/dark toggle ----
const themeCands = { headstash: null, manifest: null };
function themeApply() {
  const p = V.pickTheme([["headstash", themeCands.headstash], ["manifest", themeCands.manifest]]);
  let s = document.getElementById("hs-theme");
  if (!s) { s = document.createElement("style"); s.id = "hs-theme"; document.head.appendChild(s); }
  s.textContent = V.themeCss(p.theme); // only validated #hex values and integer px reach here
  const de = document.documentElement; de.dataset.themeSource = p.source; de.dataset.themeRejected = p.rejected.map((r) => r.source).join(",");
  for (const r of p.rejected) log(`theme from ${r.source} rejected: ${r.error}`);
}
function themeMode(m) {
  document.documentElement.dataset.theme = m; const b = $("theme-toggle");
  if (b) { b.textContent = m === "dark" ? "light" : "dark"; b.setAttribute("aria-pressed", String(m === "dark")); }
}
function themeInit(manifest) {
  themeCands.manifest = manifest && manifest.theme != null ? manifest.theme : null; themeApply();
  const mq = window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  themeMode(V.initialMode(window.localStorage, !!(mq && mq.matches)));
  if (mq && mq.addEventListener) mq.addEventListener("change", (e) => { let s = null; try { s = localStorage.getItem(V.THEME_MODE_KEY); } catch {} if (!s) themeMode(e.matches ? "dark" : "light"); });
  $("theme-toggle").onclick = () => { const m = document.documentElement.dataset.theme === "dark" ? "light" : "dark"; try { localStorage.setItem(V.THEME_MODE_KEY, m); } catch {} themeMode(m); };
}

export async function start({ files, manifest, config, urls }) {
  C = await import(urls["crypto.js"]); V = await import(urls["v2lib.js"]); ({ hex, unhex, cat, b64, unb64, te, sha256 } = C);
  themeInit(manifest);
  STEPS = JSON.parse(new TextDecoder().decode(files["steps.json"])); guard = new V.ReuseGuard(window.localStorage); book = new V.IndexBook(window.localStorage);
  cfg = { ...config, urls }; feManifest = manifest; feConfig = config; $("fe-addr").textContent = config.frontend; $("fe-ver").textContent = manifest.version;
  const url = URL.createObjectURL(new Blob([files["zk_headstash.js"]], { type: "text/javascript" }));
  Z = await import(url); await Z.default({ module_or_path: files["zk_headstash_bg.wasm"] });
  document.querySelectorAll("button.info").forEach((b) => (b.onclick = () => info(b.dataset.info)));
  $("browse").onclick = browse; $("hal-fetch").onclick = halFetch; $("hal-file").onchange = halLocal; $("mm-connect").onclick = mmConnect; $("mm-disconnect").onclick = mmDisconnect; $("key-use").onclick = keyUse;
  document.querySelectorAll("input[name=reward-mode]").forEach((r) => (r.onchange = rewardChange));
  document.querySelectorAll("input[name=dest-mode]").forEach((r) => (r.onchange = () => { $("fwd-box").hidden = destMode() !== "forward"; destOk = false; flow(); }));
  $("dest-ok").onclick = () => guarded("dst-err", async () => {
    if (destMode() === "forward") {
      const ch = $("fwd-channel").value; if (!V.validChannel(ch) || !inst || !inst.channels.some((c) => c.channel_id === ch)) throw new Error("pick an allowlisted forward channel");
      if (!zafu && !$("recv").value.trim()) throw new Error("connect Zafu or paste a one-time address first");
    }
    destOk = true;
  });
  SECS.forEach((s) => ($(`sec-${s}-toggle`).onclick = () => { if (s === cur) return; expanded.has(s) ? expanded.delete(s) : expanded.add(s); flow(); }));
  $("sheet-close").onclick = () => $("hs-sheet").close(); $("sheet-resume").onclick = () => { opener = $("sheet-resume"); openSheet(); };
  $("hs-sheet").addEventListener("close", () => { const f = opener && opener.isConnected ? opener : $("sheet-resume"); if (f && !f.hidden) f.focus(); });
  $("privacy-ack").onchange = () => flow(); $("privacy-refresh").onclick = () => guarded("claim-err", privacyRefresh);
  $("derive").onclick = derive; $("load").onclick = load; $("register").onclick = register; $("anchor-check").onclick = anchorCheck; $("claim").onclick = claim;
  $("save").onclick = save; $("export").onclick = exportWallets; $("zafu-connect").onclick = () => guarded("dst-err", zafuConnect); $("zafu-wallet").onchange = () => { if (!zafu) zafuDetect(); }; $("zafu-disconnect").onclick = () => guarded("dst-err", zafuDisconnect);
  $("zafu").onclick = () => guarded("dst-err", async () => { await zafuAddress("preview"); });
  $("reward-export").onclick = () => download("headstash-reward-key.json", JSON.stringify({ address: state.reward_addr, priv_hex: state.reward_priv }, null, 1));
  $("browse-codes").onclick = browse; $("f-token").onclick = () => filterOpen("tokens"); $("f-creator").onclick = () => filterOpen("creators");
  $("f-clear").onclick = () => { filt.tokens.clear(); filt.creators.clear(); applyFilters(); }; $("filter-q").oninput = filterRender;
  $("details-btn").onclick = detailsOpen; $("sheet-details-btn").onclick = detailsOpen; walletBind(); mmUi();
  zafuDetect().catch((e) => zafuSet("error", "Zafu client failed to load: " + e.message));
  const qp = new URLSearchParams(location.search), hs = qp.get("hs");
  let codes = qp.get("code");
  if (!codes) codes = await lcd(`/cosmwasm/wasm/v1/contract/${hs || cfg.headstash}`).then((j) => j.contract_info.code_id).catch(() => "");
  if (hs) cfg.headstash = checkTerp(hs);
  defaultHs = cfg.headstash;
  $("code-ids").value = codes || ""; $("hs-cur").textContent = "none yet (open one below)";
  themeCands.headstash = await smart({ theme: {} }).catch(() => null); themeApply(); // the default instance's theme styles the landing view
  instScope = new V.Scope(); await rewardChange(); fwdState(); anchorPoll();
  document.body.dataset.ready = "1"; log("app ready; prover loaded");
  // R13: the landing view only lists Headstash instances; nothing claim-related is shown until one is opened
  if (codes) await browse();
}
