let C, hex, unhex, cat, b64, unb64, te, sha256;
const $ = (id) => document.getElementById(id);
let cfg, Z, state = null;
const log = (...a) => { const l = $("log"); l.textContent += new Date().toISOString().slice(11, 19) + " " + a.join(" ") + "\n"; l.scrollTop = 1e9; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---- chain IO (REST + relay only; CSP connect-src enforces this) ----
async function smart(addr, q) {
  const r = await fetch(`${cfg.rest}/cosmwasm/wasm/v1/contract/${addr}/smart/${b64(te.encode(JSON.stringify(q)))}`);
  const j = await r.json(); if (!r.ok) throw new Error(j.message || r.status); return j.data;
}
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

// ---- derivation ----
async function directCtx() { const h = await sha256(te.encode(`terp-hs-direct-ctx-v1\0${cfg.chain_id}\0${cfg.headstash}`)); h[31] &= 0x3f; return h; }
async function listHash(ctx, tag, ident, notes) {
  const parts = [te.encode("terp-hs-note-list-v2"), ctx, new Uint8Array([tag]), ident, C.u16le(notes.length)];
  notes.forEach((n, i) => parts.push(new Uint8Array([n.cls]), C.u16le(i), unhex(n.k)));
  return sha256(cat(...parts));
}
async function claimWallet(s) { return await wallet(C.reduceN(await C.hkdf(unhex(s), te.encode("terp-hs-claim-key-v1"), new Uint8Array(), 32))); }
function parseRows(text) {
  const t = text.trim(); if (t.startsWith("[")) return JSON.parse(t);
  return t.split("\n").filter((l) => l.trim()).map((l) => JSON.parse(l));
}
function checkTerp(a) { const d = C.bech32Decode(a); if (d.hrp !== cfg.hrp || (d.bytes.length !== 20 && d.bytes.length !== 32)) throw new Error("not a " + cfg.hrp + " address: " + a); return a; }

async function generate() {
  const keyIn = $("key"); let priv;
  try {
    priv = unhex(keyIn.value.trim()); keyIn.value = "";
    if (!C.privOk(priv)) throw new Error("invalid secp256k1 private key");
    const f = $("elig").files[0]; if (!f) throw new Error("choose the eligibility rows file");
    const reward = checkTerp($("reward").value.trim());
    const single = $("dest-mode").value === "single" ? checkTerp($("dest").value.trim()) : null;
    const ident = C.keccak256(C.pubkey(priv).uncompressed).slice(12), id = hex(ident);
    const rows = parseRows(await f.text()).filter((r) => String(r.identity).toLowerCase().replace(/^0x/, "") === id).sort((a, b) => a.note_index - b.note_index);
    if (!rows.length) throw new Error("no eligibility rows for 0x" + id);
    const ctx = await directCtx(), ch = hex(ctx);
    log(`deriving ${rows.length} notes for 0x${id}`);
    const notes = [];
    for (const r of rows) {
      const wide = await C.hkdf(priv, te.encode("terp-hs-note-secret-v1"), cat(ctx, ident, C.u32le(r.note_index)), 64);
      const s = Z.note_secret_from_wide_hex(hex(wide)); wide.fill(0);
      const n = { c: ch, i: r.note_index, li: r.leaf_index, p: r.path, cls: r.class, amt: r.amount != null ? String(r.amount) : null, s, k: Z.membership_key_com_hex(s), n: Z.note_nullifier_hex(s, ch), st: "new" };
      n.dest = single || (await claimWallet(s)).addr; notes.push(n);
    }
    const lh = await listHash(ctx, 1, ident, notes);
    const msg = `headstash-register-notes-v3:${ch}:${hex(lh)}:${notes.length}:${reward}`, m = te.encode(msg);
    const sig = await C.sign(priv, C.keccak256(cat(te.encode("\x19Ethereum Signed Message:\n" + m.length), m)));
    state = { v: 1, chain_id: cfg.chain_id, contract: cfg.headstash, identity: id, created: Math.floor(Date.now() / 1000), reward_addr: reward,
      registration: { list_hash: hex(lh), signature: hex(cat(sig.r, sig.s, new Uint8Array([27 + (sig.rec & 1)]))) }, notes };
    log(`generated ${notes.length} notes; list_hash ${hex(lh).slice(0, 16)}…; key wiped`);
    render(); await save();
  } catch (e) { log("ERROR", e.message); } finally { if (priv) priv.fill(0); keyIn.value = ""; }
}

// ---- notes file ----
const canon = (o) => Array.isArray(o) ? "[" + o.map(canon).join(",") + "]" : o && typeof o === "object" ? "{" + Object.keys(o).sort().filter((k) => o[k] !== undefined).map((k) => JSON.stringify(k) + ":" + canon(o[k])).join(",") + "}" : JSON.stringify(o);
async function fileBytes() {
  const o = { ...state }; delete o.sha256; o.sha256 = hex(await sha256(te.encode(canon(o))));
  let out = JSON.stringify(o, null, 1); const pass = $("pass").value;
  if (pass) {
    const salt = crypto.getRandomValues(new Uint8Array(16)), iv = crypto.getRandomValues(new Uint8Array(12));
    const k = await crypto.subtle.deriveKey({ name: "PBKDF2", hash: "SHA-256", salt, iterations: 600000 }, await crypto.subtle.importKey("raw", te.encode(pass), "PBKDF2", false, ["deriveKey"]), { name: "AES-GCM", length: 256 }, false, ["encrypt"]);
    out = JSON.stringify({ v: 1, enc: "pbkdf2-aesgcm", salt: b64(salt), iv: b64(iv), ct: b64(new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, k, te.encode(out)))) });
  }
  return out;
}
async function save() { const a = document.createElement("a"); a.href = URL.createObjectURL(new Blob([await fileBytes()], { type: "application/json" })); a.download = "headstash-notes.v1.json"; document.body.appendChild(a); a.click(); a.remove(); log("notes file saved"); }
async function load() {
  try {
    let o = JSON.parse(await $("nf").files[0].text());
    if (o.enc === "pbkdf2-aesgcm") {
      const k = await crypto.subtle.deriveKey({ name: "PBKDF2", hash: "SHA-256", salt: unb64(o.salt), iterations: 600000 }, await crypto.subtle.importKey("raw", te.encode($("pass").value), "PBKDF2", false, ["deriveKey"]), { name: "AES-GCM", length: 256 }, false, ["decrypt"]);
      o = JSON.parse(new TextDecoder().decode(await crypto.subtle.decrypt({ name: "AES-GCM", iv: unb64(o.iv) }, k, unb64(o.ct))));
    }
    const want = o.sha256; delete o.sha256;
    if (o.v !== 1 || hex(await sha256(te.encode(canon(o)))) !== want) throw new Error("notes file checksum mismatch or unknown version");
    if (o.chain_id !== cfg.chain_id || o.contract !== cfg.headstash) throw new Error("notes file is for another chain/contract");
    state = o; log(`loaded ${o.notes.length} notes (checksum ok)`); render();
  } catch (e) { log("ERROR load:", e.message); }
}

// ---- register / claim ----
async function register() {
  const todo = state.notes.filter((n) => n.st === "new"); if (!todo.length) return log("nothing to register");
  const all = state.notes, list = all.map((n) => ({ class: n.cls, key_com: b64(unhex(n.k)) }));
  for (let start = 0; start < all.length; start += 32) {
    const chunk = all.slice(start, start + 32); if (chunk.every((n) => n.st !== "new")) continue;
    const first = start === 0;
    const body = { signer: { evm: { address: b64(unhex(state.identity)) } }, signature: first ? b64(unhex(state.registration.signature)) : null, list, start,
      eligibility: chunk.map((n) => ({ index: n.li, path: n.p.map((x) => b64(unhex(x))) })), reward_addr: first ? state.reward_addr : null };
    const w = await wallet(C.reduceN(crypto.getRandomValues(new Uint8Array(32))));
    try {
      const g = await grant(w.addr, "register", body);
      const res = await execTx(w, { register_notes: body }, cfg.register_gas, g.granter);
      if (res.code === 0 || /already registered/i.test(res.raw_log || "")) { chunk.forEach((n) => { n.st = "registered"; n.rtx = res.txhash; }); log(`registered notes ${start}..${start + chunk.length - 1} tx ${res.txhash}`); }
      else { log(`register chunk ${start} failed code ${res.code}: ${(res.raw_log || "").slice(0, 200)}`); break; }
    } catch (e) { log("ERROR register:", e.message); break; }
    render();
  }
  render(); await save();
}
async function claim() {
  const rt = await smart(cfg.headstash, { registration_tree: {} });
  if (!rt.latest_anchor) return log("no accepted anchor yet: wait for the next anchor, then claim");
  const root = rt.latest_anchor.root, rootHex = hex(unb64(root));
  for (const n of state.notes.filter((x) => x.st === "registered" || x.st === "submitted")) {
    let pth;
    try { pth = await smart(cfg.headstash, { registration_path: { key_com: b64(unhex(n.k)), anchor: root } }); }
    catch (e) { log(`note ${n.i}: not in the latest anchor yet (${e.message.slice(0, 80)})`); continue; }
    try {
      log(`note ${n.i}: proving…`); await sleep(30);
      const t0 = performance.now();
      const pr = JSON.parse(Z.prove_note_witness_browser(JSON.stringify({ secret_hex: n.s, ctx_hex: n.c, root_hex: rootHex, destination: n.dest, amount: String(pth.amount), index: pth.index, siblings_hex: pth.siblings })));
      const p = pr.body.split(":");
      if (p[3] !== n.n) throw new Error("nullifier mismatch between notes file and proof");
      n.prove_ms = Math.round(performance.now() - t0); n.amt = p[5];
      const msg = { root, nullifier: b64(unhex(p[3])), amount: Number(p[5]), proof: b64(unhex(pr.proof_hex)) };
      const w = await claimWallet(n.s), g = await grant(w.addr, "claim", msg);
      n.st = "submitted"; render();
      const res = await execTx(w, { claim_note: msg }, cfg.claim_gas, g.granter); n.tx = res.txhash;
      if (res.code === 0 || /nullifier.*(spent|used)|already (claimed|spent)/i.test(res.raw_log || "")) { n.st = "spent"; log(`note ${n.i}: claimed ${p[5]} → ${n.dest} (proof ${n.prove_ms} ms) tx ${res.txhash}`); }
      else log(`note ${n.i}: claim failed code ${res.code}: ${(res.raw_log || "").slice(0, 200)}`);
    } catch (e) { log(`ERROR note ${n.i}:`, e.message); }
    render();
  }
  await save();
}
async function exportWallets() {
  const out = []; for (const n of state.notes) { const w = await claimWallet(n.s); out.push({ note: n.i, address: w.addr, priv_hex: hex(w.priv) }); }
  const a = document.createElement("a"); a.href = URL.createObjectURL(new Blob([JSON.stringify(out, null, 1)], { type: "application/json" })); a.download = "headstash-claim-wallets.json"; a.click();
}
function render() {
  if (!state) return; const by = {}; state.notes.forEach((n) => (by[n.st] = (by[n.st] || 0) + 1));
  $("summary").textContent = `0x${state.identity}: ${state.notes.length} notes — ` + Object.entries(by).map(([k, v]) => `${k} ${v}`).join(", ");
  $("summary").dataset.counts = JSON.stringify(by);
  $("register").disabled = !by.new; $("claim").disabled = !(by.registered || by.submitted); $("save").disabled = false; $("export").disabled = false;
  $("tbl").innerHTML = "<tr><th>#</th><th>class</th><th>status</th><th>destination</th></tr>" + state.notes.map((n) => `<tr><td>${n.i}</td><td>${n.cls}</td><td>${n.st}</td><td>${n.dest}</td></tr>`).join("");
}
function penumbra() {
  const a = $("pen").value.trim();
  try { const d = C.bech32Decode(a, true); if (d.hrp !== "penumbra") throw new Error("hrp must be penumbra"); $("pen-out").textContent = JSON.stringify({ valid: true, note: "PREVIEW ONLY. Nothing is sent. The channel and timeout are not set yet.", ibc_transfer: { source_port: "transfer", source_channel: "<TBD>", denom: cfg.denom, receiver: a, timeout: "<TBD>" }, tip: "Use a fresh diversified address for each note so claims stay unlinkable." }, null, 1); }
  catch (e) { $("pen-out").textContent = "invalid Penumbra address: " + e.message; }
}

export async function start({ files, manifest, config, urls }) {
  C = await import(urls["crypto.js"]); ({ hex, unhex, cat, b64, unb64, te, sha256 } = C);
  cfg = config; $("fe-addr").textContent = config.frontend; $("fe-ver").textContent = manifest.version;
  const url = URL.createObjectURL(new Blob([files["zk_headstash.js"]], { type: "text/javascript" }));
  Z = await import(url); await Z.default({ module_or_path: files["zk_headstash_bg.wasm"] });
  $("how").onclick = () => ($("modal").style.display = "block"); $("close").onclick = () => ($("modal").style.display = "none");
  $("gen").onclick = generate; $("load").onclick = load; $("register").onclick = register; $("claim").onclick = claim; $("save").onclick = save; $("export").onclick = exportWallets; $("pen-check").onclick = penumbra;
  $("pen-detect").textContent = window[Symbol.for("penumbra")] ? "Zafu/Prax wallet detected." : "No Penumbra wallet (Zafu/Prax) detected. You can still paste an address.";
  document.body.dataset.ready = "1"; log("app ready; prover loaded");
}
