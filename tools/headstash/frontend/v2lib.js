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

// ---- R5: Zafu diversified-address index allocation (see docs/V2-SPEC.md §Zafu) ----
// Penumbra AddressIndex = {account: u32, randomizer: 12 bytes}; diversifier = AES-128(dk, account_le32 || randomizer).
// Every claim gets a FRESH index: a CSPRNG randomizer (never all-zero, which is the account's default address),
// recorded locally together with the address the wallet proved for it, so no index or address is ever reused here.
export const ZAFU_INDEX_KEY = "terp-hs-zafu-indices-v1";
export const RANDOMIZER_LEN = 12;
export function parseAccount(v) {
  const s = String(v ?? "").trim(); if (!/^\d{1,10}$/.test(s)) throw new Error("account must be a whole number");
  const n = Number(s); if (n > 0xffffffff) throw new Error("account must fit in u32"); return n;
}
export class IndexBook {
  constructor(storage, rng = (b) => crypto.getRandomValues(b)) { this.s = storage; this.rng = rng; }
  _all() { try { const a = JSON.parse(this.s.getItem(ZAFU_INDEX_KEY) || "[]"); return Array.isArray(a) ? a : []; } catch { return []; } }
  static key(account, randomizer) { return account + ":" + hexOf(randomizer); }
  /** A fresh {account, randomizer}: 12 random bytes, non-zero, never handed out before on this device. */
  allocate(account) {
    account = parseAccount(account); const used = new Set(this._all().map((r) => r.k));
    for (let i = 0; i < 8; i++) {
      const r = this.rng(new Uint8Array(RANDOMIZER_LEN));
      if (!(r instanceof Uint8Array) || r.length !== RANDOMIZER_LEN) throw new Error("rng must fill 12 bytes");
      if (r.every((x) => x === 0) || used.has(IndexBook.key(account, r))) continue;
      return { account, randomizer: Uint8Array.from(r) };
    }
    throw new Error("could not draw a fresh address index (rng is broken)");
  }
  /** Record the index and the address the wallet proved for it. Refuses any reuse of either. */
  record(idx, address, owner) {
    if (!idx || !(idx.randomizer instanceof Uint8Array) || idx.randomizer.length !== RANDOMIZER_LEN || idx.randomizer.every((x) => x === 0)) throw new Error("refusing a zero / malformed randomizer");
    const a = this._all(), k = IndexBook.key(parseAccount(idx.account), idx.randomizer), addr = String(address).trim().toLowerCase();
    if (a.some((r) => r.k === k)) throw new ReuseError("address index already used on this device");
    if (a.some((r) => r.a === addr)) throw new ReuseError("address already used on this device");
    a.push({ k, a: addr, owner: owner || null, t: Date.now() }); this.s.setItem(ZAFU_INDEX_KEY, JSON.stringify(a)); return true;
  }
  count() { return this._all().length; }
}
/** Map a Penumbra provider failure (Error.cause = PenumbraRequestFailure) to user text. */
export function zafuErrorText(e) {
  const c = e && (e.cause && typeof e.cause === "object" ? e.cause.cause : e.cause), m = String((e && e.message) || e);
  if (c === "Denied") return "Zafu: connection was declined. Approve this site in Zafu to request an address.";
  if (c === "NeedsLogin") return "Zafu is locked: unlock it, then connect again.";
  if (c === "NotHandled") return "Zafu did not handle the request (site not allowed or extension reloaded): reload the page and retry.";
  if (/network not enabled|network not active/.test(m)) return "Zafu: enable and select the Penumbra network in Zafu, then retry.";
  if (/timed out/i.test(m)) return "Zafu did not answer in time: open Zafu and check for a pending approval.";
  return "Zafu error: " + m;
}

// ---- R12: on-chain theme tokens (same rules as cw-headstash / cw-headstash-frontend theme.rs) ----
export const THEME_TOKENS = ["bg", "surface", "surfaceAlt", "text", "textMuted", "border", "borderStrong", "accent", "onAccent", "focus"];
export const THEME_MODES = ["light", "dark"];
export const THEME_MODE_KEY = "terp-hs-theme-mode-v1";
export const RADIUS_MAX = 24;
const HEXC = /^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/;
export const BUILTIN_THEME = Object.freeze({
  light: { bg: "#FAFAF7", surface: "#FFFFFF", surfaceAlt: "#F2F2EE", text: "#111111", textMuted: "#555555", border: "#CCCCCC", borderStrong: "#888888", accent: "#1A5FB4", onAccent: "#FFFFFF", focus: "#1A5FB4" },
  dark: { bg: "#121212", surface: "#1E1E1E", surfaceAlt: "#2A2A2A", text: "#EEEEEE", textMuted: "#AAAAAA", border: "#3A3A3A", borderStrong: "#777777", accent: "#8AB4F8", onAccent: "#111111", focus: "#8AB4F8" },
  radius_px: 6,
});
// Azure Calcium: the manifest default and the terpy/thioly headstash theme (frontend/themes/azure-calcium.json).
export const AZURE_CALCIUM = Object.freeze({
  light: { bg: "#EFF7FF", surface: "#F9FCFF", surfaceAlt: "#DEEFFF", text: "#233E58", textMuted: "#325A80", border: "#C8DBEF", borderStrong: "#A0D0FF", accent: "#4078AC", onAccent: "#FFFFFF", focus: "#4078AC" },
  dark: { bg: "#05101B", surface: "#0B1B2B", surfaceAlt: "#192A3A", text: "#E2EDF9", textMuted: "#A6B8CA", border: "#253748", borderStrong: "#4078AC", accent: "#76BBFC", onAccent: "#132230", focus: "#76BBFC" },
});
const own = (o, k) => Object.prototype.hasOwnProperty.call(o, k);
const plain = (o) => o !== null && typeof o === "object" && !Array.isArray(o) && Object.getPrototypeOf(o) === Object.prototype;
/** Throws on anything outside the whitelist; returns a fresh, frozen copy (never the input object). */
export function validateTheme(t) {
  if (!plain(t)) throw new Error("theme: not an object");
  for (const k of Object.keys(t)) if (!["light", "dark", "radius_px"].includes(k)) throw new Error("theme: unknown field " + JSON.stringify(k).slice(0, 40));
  const out = { light: {}, dark: {} }; let n = 0;
  for (const m of THEME_MODES) {
    const v = own(t, m) ? t[m] : {};
    if (!plain(v)) throw new Error(`theme.${m}: not an object`);
    const ks = Object.keys(v); if (ks.length > THEME_TOKENS.length) throw new Error(`theme.${m}: too many tokens`);
    for (const k of ks) {
      if (!THEME_TOKENS.includes(k)) throw new Error(`theme.${m}: unknown token ` + JSON.stringify(k).slice(0, 40));
      const c = v[k]; if (typeof c !== "string" || !HEXC.test(c)) throw new Error(`theme.${m}.${k}: colour must be #RGB or #RRGGBB`);
      out[m][k] = c; n++;
    }
  }
  if (own(t, "radius_px") && t.radius_px !== null) {
    const r = t.radius_px; if (!Number.isInteger(r) || r < 0 || r > RADIUS_MAX) throw new Error("theme.radius_px: integer 0.." + RADIUS_MAX);
    out.radius_px = r;
  }
  if (!n) throw new Error("theme: no tokens");
  return Object.freeze(out);
}
/** First valid theme in order (headstash > manifest > built-in); missing tokens fall back to the built-in. */
export function pickTheme(cands) {
  const rejected = [];
  for (const [source, t] of cands) {
    if (t === null || t === undefined) continue;
    try { const v = validateTheme(t); return { source, theme: mergeTheme(v), rejected }; } catch (e) { rejected.push({ source, error: e.message }); }
  }
  return { source: "builtin", theme: mergeTheme(BUILTIN_THEME), rejected };
}
export function mergeTheme(v) {
  const o = { light: { ...BUILTIN_THEME.light, ...v.light }, dark: { ...BUILTIN_THEME.dark, ...v.dark }, radius_px: v.radius_px ?? BUILTIN_THEME.radius_px };
  return o;
}
/** CSS text with --hs-* custom properties; re-validates every value, so nothing else can reach the stylesheet. */
export function themeCss(t) {
  const v = validateTheme(t), r = v.radius_px ?? BUILTIN_THEME.radius_px;
  const blk = (m) => THEME_TOKENS.filter((k) => own(v[m], k)).map((k) => `--hs-${k}:${v[m][k]}`).concat(`--hs-radius:${r}px`).join(";");
  return `:root[data-theme=light]{${blk("light")}}\n:root[data-theme=dark]{${blk("dark")}}\n`;
}
export function initialMode(storage, prefersDark) {
  let s = null; try { s = storage && storage.getItem(THEME_MODE_KEY); } catch {}
  return s === "light" || s === "dark" ? s : (prefersDark ? "dark" : "light");
}
// WCAG 2.x relative luminance / contrast ratio
export function luminance(c) {
  if (!HEXC.test(c)) throw new Error("bad colour");
  let h = c.slice(1); if (h.length === 3) h = [...h].map((x) => x + x).join("");
  const [r, g, b] = [0, 2, 4].map((i) => { const x = parseInt(h.slice(i, i + 2), 16) / 255; return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4; });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
export function contrast(a, b) { const [x, y] = [luminance(a), luminance(b)].sort((p, q) => p - q); return (y + 0.05) / (x + 0.05); }

// ---- R13: privacy set (registered − claimed) and the delayed-claim relay extension point ----
/** Floor of the suggested privacy set; the effective threshold is max(PRIVACY_FLOOR, PRIVACY_ANCHOR_FACTOR × min_anonymity_set). */
export const PRIVACY_FLOOR = 64;
export const PRIVACY_ANCHOR_FACTOR = 4;
const count = (x, what) => {
  const n = typeof x === "string" && /^\d{1,16}$/.test(x) ? Number(x) : x;
  if (!Number.isSafeInteger(n) || n < 0) throw new Error(`privacy set: bad ${what} count from chain`);
  return n;
};
export function privacyThreshold(minAnon) {
  const m = minAnon == null ? 0 : count(minAnon, "min_anonymity_set");
  return Math.max(PRIVACY_FLOOR, PRIVACY_ANCHOR_FACTOR * m);
}
/**
 * From the on-chain queries `registration_tree` (count = registered leaves) and `direct_claim` (claimed = ClaimNote +
 * ClaimNoteForward successes). Lane/public claims are not counted by the contract (see V2-SPEC R13).
 */
export function privacySet(rt, dc) {
  if (!rt || typeof rt !== "object") throw new Error("privacy set: no registration_tree response");
  if (!dc || typeof dc !== "object") throw new Error("privacy set: no direct_claim response");
  const registered = count(rt.count, "registered"), claimed = count(dc.claimed ?? 0, "claimed");
  if (claimed > registered) throw new Error("privacy set: claimed exceeds registered");
  const anchored = rt.latest_anchor ? count(rt.latest_anchor.leaf_count, "anchored") : 0;
  const threshold = privacyThreshold(rt.min_anonymity_set), set = registered - claimed;
  return { registered, claimed, set, anchored, threshold, small: set < threshold };
}
export function privacyAdvice(p) {
  if (p.small) return `Only ${p.set} registered note${p.set === 1 ? " is" : "s are"} still unclaimed. Your claim hides among them, so claiming now gives a small privacy set. We suggest waiting until it reaches at least ${p.threshold}. You can still claim now after confirming below.`;
  return `${p.set} registered notes are still unclaimed (suggested minimum ${p.threshold}): your claim blends into a healthy privacy set.`;
}
/** Claim is allowed when the set is not small, or the user explicitly confirmed. Unknown set (query failed) also requires the confirm. */
export const privacyGateOpen = (p, ack) => !!ack || (!!p && !p.small);
/**
 * EXTENSION POINT (not implemented): delayed-claim relay.
 * A relay receives an already-proved claim and submits it later so the claim time does not link to the user's session.
 * @typedef {Object} DelayedClaimRelay
 * @property {string} id         stable identifier
 * @property {string} name       shown to the user
 * @property {string} origin     https origin; MUST also be in the loader CSP connect-src
 * @property {(claim: {msg: object, kind: "claim_note"|"claim_note_forward", chain_id: string, contract: string},
 *             opts: {not_before: number, not_after: number}) => Promise<{ticket: string}>} submit
 * @property {(ticket: string) => Promise<{state: "queued"|"submitted"|"failed", txhash?: string}>} status
 * This build ships NO relay: the list is empty and the page never shows a relay option.
 */
export const DELAYED_CLAIM_RELAYS = Object.freeze([]);
export function delayedRelays(list = DELAYED_CLAIM_RELAYS) {
  return list.filter((r) => r && typeof r.id === "string" && typeof r.name === "string" && /^https:\/\/[^/\s]+$/.test(r.origin || "") && typeof r.submit === "function" && typeof r.status === "function");
}

// ---- fix2: display helpers (pure, unit-tested) ----
/** terp1yyca…jf8 style: head + ellipsis + tail; short strings pass through. */
export function shortAddr(a, head = 9, tail = 3) { const s = String(a ?? ""); return s.length > head + tail + 1 ? `${s.slice(0, head)}…${s.slice(-tail)}` : s; }
/** Micro-denom amounts as display units: 1000000000 uterp -> "1,000 TERP"; other denoms unchanged. */
export function fmtAmount(amount, denom) {
  const d = String(denom || ""), n = Number(amount);
  if (!Number.isFinite(n)) return `${amount} ${d}`.trim();
  if (/^u[a-z]{2,}$/.test(d)) return `${(n / 1e6).toLocaleString("en-US", { maximumFractionDigits: 6 })} ${d.slice(1).toUpperCase()}`;
  return `${n.toLocaleString("en-US")} ${d}`.trim();
}
/**
 * Why claims are (not yet) possible, from public contract state only:
 * registration_tree {count, min_anonymity_set, next_anchor_height, latest_anchor} and the latest block height.
 * state: "waiting-set" (count < min_anonymity_set; accept_anchor fails "anonymity set"), "due" (an anchor may be accepted now),
 * "waiting-height" (next_anchor_height not reached), "accepted" (latest anchor covers every registration), "stale" (accepted but newer registrations wait).
 */
export function anchorStatus(rt, height, interval) {
  if (!rt || typeof rt !== "object") throw new Error("anchor status: no registration_tree response");
  const count = Number(rt.count) || 0, min = Number(rt.min_anonymity_set) || 0, next = Number(rt.next_anchor_height) || 0;
  const h = height == null || height === "" ? null : Number(height), la = rt.latest_anchor || null, leaves = la ? Number(la.leaf_count) || 0 : 0;
  const iv = interval == null ? null : Number(interval);
  const base = { count, min, next, height: h, interval: iv, accepted: !!la, leaves, need: Math.max(0, min - count) };
  if (count < min) return { ...base, state: "waiting-set", reason: `Anchor blocked: ${count} of ${min} registrations needed for the minimum anonymity set (${min - count} more).`,
    eta: `The next anchor can be accepted once ${min - count} more note${min - count === 1 ? " is" : "s are"} registered${iv ? `; after that, at most ${iv} block${iv === 1 ? "" : "s"} apart` : ""}.` };
  if (la && leaves >= count) return { ...base, state: "accepted", reason: `Anchor accepted: covers all ${leaves} registrations.`, eta: "" };
  const due = h == null || h >= next;
  return { ...base, state: due ? (la ? "stale" : "due") : "waiting-height",
    reason: due ? `${count} registrations meet the minimum of ${min}: an anchor can be accepted now${la ? ` (latest anchor covers ${leaves})` : ""}.` : `Next anchor allowed at block ${next} (now ${h}).`,
    eta: due ? "Waiting for the next accept_anchor call (anyone may send it)." : `About ${next - h} block${next - h === 1 ? "" : "s"} to go.` };
}
/** Claim dependency chain for the step chips: Registered -> Anchor accepted -> Claim. Each step: done | current | todo. */
export function claimChain(notes, anchor) {
  const ns = notes || [], reg = ns.some((n) => n.st !== "new"), spent = ns.length > 0 && ns.every((n) => n.st === "spent");
  const anc = !!anchor && anchor.accepted;
  const s = [["registered", "Registered", reg], ["anchor", "Anchor accepted", reg && anc], ["claim", "Claim", spent]];
  let cur = false;
  return s.map(([id, label, done]) => { const st = done ? "done" : cur ? "todo" : ((cur = true), "current"); return { id, label, state: st }; });
}
