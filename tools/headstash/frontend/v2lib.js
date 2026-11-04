// Headstash claim page v2: pure helpers (no DOM, no network). Unit-tested with `node --test` (test/v2lib.test.mjs).
const te = new TextEncoder();
export const hexOf = (b) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
export const fromHex = (h) => { const s = String(h).replace(/^0x/, ""); if (s.length % 2 || /[^0-9a-f]/i.test(s)) throw new Error("bad hex"); return Uint8Array.from(s.match(/../g) || [], (x) => parseInt(x, 16)); };
export const sha256 = async (b) => new Uint8Array(await crypto.subtle.digest("SHA-256", b));
const b64u = (b) => { let s = ""; for (const x of b) s += String.fromCharCode(x); return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""); };
export const b64ToHex = (s) => hexOf(Uint8Array.from(atob(s), (c) => c.charCodeAt(0)));

// ---- R1: HAL source resolution + verification ----
export const DEFAULT_GATEWAYS = ["https://ipfs.io", "https://dweb.link", "https://w3s.link"];
const LOOPBACK = /^(127\.0\.0\.1|localhost|\[::1\])(:\d+)?$/i;
/** Candidate fetch URLs for a contract `hal_source{}` (uri as stored on chain). */
export function resolveHal(src, gateways = DEFAULT_GATEWAYS) {
  if (!src || !src.uri) throw new Error("this instance has no HAL source set");
  const u = String(src.uri), l = u.toLowerCase(), out = [];
  if (l.startsWith("https://")) out.push(u);
  else if (l.startsWith("ipfs://")) gateways.forEach((g) => out.push(`${g}/ipfs/${u.slice(7)}`));
  else if (l.startsWith("ipns://")) gateways.forEach((g) => out.push(`${g}/ipns/${u.slice(7)}`));
  else if (l.startsWith("ar://")) out.push(`https://arweave.net/${u.slice(5)}`);
  else if (/^[a-z][a-z0-9+.-]*:\/\//.test(l)) throw new Error("unsupported HAL uri scheme: " + u);
  else { const host = u.split("/")[0]; out.push((LOOPBACK.test(host) ? "http://" : "https://") + u); }
  if (src.cid && !l.startsWith("ipfs://")) gateways.forEach((g) => out.push(`${g}/ipfs/${src.cid}`));
  return [...new Set(out)];
}
const B32 = "abcdefghijklmnopqrstuvwxyz234567";
function b32dec(s) { let acc = 0, bits = 0; const o = []; for (const c of s) { const v = B32.indexOf(c); if (v < 0) throw new Error("bad base32"); acc = (acc << 5) | v; bits += 5; if (bits >= 8) { bits -= 8; o.push((acc >> bits) & 255); } } return Uint8Array.from(o); }
function b32enc(b) { let acc = 0, bits = 0, o = ""; for (const x of b) { acc = (acc << 8) | x; bits += 8; while (bits >= 5) { bits -= 5; o += B32[(acc >> bits) & 31]; } } if (bits) o += B32[(acc << (5 - bits)) & 31]; return o; }
function uvar(b, i) { let v = 0, s = 0; for (;;) { const x = b[i++]; if (x === undefined) throw new Error("truncated varint"); v += (x & 127) * 2 ** s; s += 7; if (!(x & 128)) return [v, i]; } }
/** Parse a CIDv1 (multibase 'b' base32). Returns {codec, hash, digest}. CIDv0 (Qm…) is dag-pb. */
export function parseCid(cid) {
  const c = String(cid);
  if (/^Qm[1-9A-HJ-NP-Za-km-z]{44}$/.test(c)) return { version: 0, codec: 0x70, hash: 0x12, digest: null };
  if (c[0] !== "b") throw new Error("only base32 CIDv1 ('b…') or CIDv0 supported");
  const b = b32dec(c.slice(1)); let i = 0, ver, codec, h, len;
  [ver, i] = uvar(b, i); [codec, i] = uvar(b, i); [h, i] = uvar(b, i); [len, i] = uvar(b, i);
  if (ver !== 1) throw new Error("not a CIDv1"); if (b.length - i !== len) throw new Error("bad multihash length");
  return { version: 1, codec, hash: h, digest: b.slice(i) };
}
export const rawCid = (digest) => "b" + b32enc(Uint8Array.from([1, 0x55, 0x12, 32, ...digest]));
/** Verify fetched HAL bytes against the on-chain pin. Throws on any mismatch; returns the methods that passed. */
export async function verifyHal(bytes, src) {
  const d = await sha256(bytes), dh = hexOf(d), passed = [];
  const pin = src.sha256 ? (typeof src.sha256 === "string" && /^[0-9a-f]{64}$/i.test(src.sha256) ? src.sha256.toLowerCase() : null) : null;
  if (src.sha256) { if (!pin) throw new Error("hal_source.sha256 is not 32-byte hex"); if (pin !== dh) throw new Error(`HAL sha256 mismatch: got ${dh}, chain says ${pin}`); passed.push("sha256"); }
  if (src.cid) {
    const c = parseCid(src.cid);
    if (c.codec === 0x55 && c.hash === 0x12) { if (hexOf(c.digest) !== dh) throw new Error("HAL CID (raw, sha2-256) mismatch"); passed.push("cid-raw"); }
    else if (!passed.length) throw new Error("HAL CID is not a raw-leaf sha2-256 CID (dag-pb/UnixFS cannot be verified in-browser yet) and no sha256 is pinned: refusing");
  }
  if (!passed.length) throw new Error("hal_source has neither sha256 nor cid: refusing unverified data");
  return { sha256: dh, passed };
}
/** Parse + structurally check the HAL document and bind it to the chain's eligibility root (hex). */
export function parseHal(bytes, chainRootHex) {
  let h; try { h = JSON.parse(new TextDecoder().decode(bytes)); } catch { throw new Error("HAL is not JSON"); }
  if (h.kind !== "terp-headstash-hal" || h.v !== 1) throw new Error("unknown HAL format");
  if (!/^[0-9a-f]{64}$/.test(h.eligibility_root || "")) throw new Error("HAL eligibility_root missing");
  if (chainRootHex != null && h.eligibility_root !== chainRootHex.toLowerCase()) throw new Error(`HAL is for another eligibility root (${h.eligibility_root.slice(0, 12)}… vs chain ${chainRootHex.slice(0, 12)}…)`);
  if (!Array.isArray(h.rows)) throw new Error("HAL rows missing");
  const by = new Map();
  for (const r of h.rows) {
    if (typeof r.identity !== "string" || !Number.isInteger(r.note_index) || !Number.isInteger(r.leaf_index) || !Array.isArray(r.path) || r.path.length !== h.depth) throw new Error("malformed HAL row");
    const id = r.identity.toLowerCase().replace(/^0x/, ""); if (!by.has(id)) by.set(id, []); by.get(id).push(r);
  }
  for (const v of by.values()) v.sort((a, b) => a.note_index - b.note_index);
  return { hal: h, byIdentity: by };
}

// ---- R5: forward destination (must equal cw-headstash forward.rs forward_destination) ----
export const FWD_DOMAIN = "terp-headstash-forward-v1";
const lp = (s) => { const b = te.encode(s), o = new Uint8Array(4 + b.length); new DataView(o.buffer).setUint32(0, b.length); o.set(b, 4); return o; };
export async function forwardDestination(chainId, contract, t) {
  const parts = [FWD_DOMAIN, chainId, contract, t.channel_id, t.receiver, t.memo || "", t.fallback].map(lp);
  const all = new Uint8Array(parts.reduce((a, p) => a + p.length, 0)); let o = 0; for (const p of parts) { all.set(p, o); o += p.length; }
  return "hsfwd1" + b64u(await sha256(all));
}
export const validChannel = (c) => /^channel-\d{1,10}$/.test(c);
export const validReceiver = (r) => typeof r === "string" && r.length > 0 && r.length <= 256 && [...r].every((c) => c.charCodeAt(0) >= 0x21 && c.charCodeAt(0) <= 0x7e);
/** Penumbra address: bech32m, hrp "penumbra", 80-byte payload. `decode` = crypto.js bech32Decode. */
export function checkPenumbra(a, decode) {
  const d = decode(a, true); if (d.hrp !== "penumbra") throw new Error("hrp must be penumbra"); if (d.bytes.length !== 80) throw new Error("payload must be 80 bytes");
  return a;
}
const CS = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
function pm(v) { const G = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]; let c = 1; for (const x of v) { const b = c >> 25; c = ((c & 0x1ffffff) << 5) ^ x; for (let i = 0; i < 5; i++) if ((b >> i) & 1) c ^= G[i]; } return c; }
export function bech32mEncode(hrp, bytes) {
  const d = []; let acc = 0, bits = 0; for (const x of bytes) { acc = (acc << 8) | x; bits += 8; while (bits >= 5) { bits -= 5; d.push((acc >> bits) & 31); } } if (bits) d.push((acc << (5 - bits)) & 31);
  const hx = [...[...hrp].map((c) => c.charCodeAt(0) >> 5), 0, ...[...hrp].map((c) => c.charCodeAt(0) & 31)];
  const p = pm([...hx, ...d, 0, 0, 0, 0, 0, 0]) ^ 0x2bc830a3;
  return hrp + "1" + [...d, ...[0, 1, 2, 3, 4, 5].map((i) => (p >> (5 * (5 - i))) & 31)].map((x) => CS[x]).join("");
}

// ---- R4/R5: local never-reuse record ----
export class ReuseError extends Error {}
export const GUARD_KEY = "terp-hs-used-addresses-v1";
/** Records sha256(address) -> {kind, owner}. An address may only ever serve one owner (one note nullifier / attempt). */
export class ReuseGuard {
  constructor(storage) { this.s = storage; }
  _all() { try { return JSON.parse(this.s.getItem(GUARD_KEY) || "{}"); } catch { return {}; } }
  async _k(addr) { return hexOf(await sha256(te.encode(String(addr).trim().toLowerCase()))); }
  async check(addr, owner) { const r = this._all()[await this._k(addr)]; if (r && r.owner !== owner) throw new ReuseError(`address already used (${r.kind}) on this device: pick a fresh one`); return !r; }
  async use(addr, kind, owner) { await this.check(addr, owner); const a = this._all(), k = await this._k(addr); if (!a[k]) { a[k] = { kind, owner, t: Date.now() }; this.s.setItem(GUARD_KEY, JSON.stringify(a)); } return true; }
  count() { return Object.keys(this._all()).length; }
}
export class MemStorage { constructor() { this.m = new Map(); } getItem(k) { return this.m.has(k) ? this.m.get(k) : null; } setItem(k, v) { this.m.set(k, String(v)); } }

// ---- R6: step modals must cover every form step ----
export function missingSteps(html, steps) {
  const ids = [...html.matchAll(/data-step="([a-z-]+)"/g)].map((m) => m[1]), set = new Set(ids), out = [];
  for (const id of set) { const s = steps[id]; if (!s || !s.title || !s.what || !s.reveals || !s.risks || !html.includes(`data-info="${id}"`)) out.push(id); }
  return out;
}

// ---- R7: instance listing ----
export const codeContractsPath = (codeId, key) => `/cosmwasm/wasm/v1/code/${encodeURIComponent(codeId)}/contracts?pagination.limit=100${key ? "&pagination.key=" + encodeURIComponent(key) : ""}`;
export function parseNumList(s) { return String(s || "").split(/[\s,]+/).filter(Boolean).map((x) => { if (!/^\d{1,10}$/.test(x)) throw new Error("code id must be a number: " + x); return x; }); }
