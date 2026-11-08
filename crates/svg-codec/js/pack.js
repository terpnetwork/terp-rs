/*! svg-launchpad/pack.js: SVP1 compact template codec (encoder + decoder).
 * Mirror of the Rust crate cw-svg-pack. Encodings are byte-identical across both.
 * Wire: "SVP" 0x01 | var table | template ops | 0x00.
 */
import { DICT } from './pack-dict.js';

const MAGIC = [0x53, 0x56, 0x50, 0x01];
export const MAX_OUT = 42 * 1024;
const OP = { END: 0, LIT: 1, DICT: 2, COPY: 3, VAR: 4, NUM: 5, NL: 6 };
const te = new TextEncoder();
const td = new TextDecoder('utf-8', { fatal: true });
const DICT_B = DICT.map((s) => te.encode(s));
const NUM_RE = /^-?(0|[1-9][0-9]*)(\.[0-9]+)?$/;

function vlen(n) { n = BigInt(n); let l = 1; while (n >= 0x80n) { n >>= 7n; l++; } return l; }
function wv(out, n) { n = BigInt(n); while (n >= 0x80n) { out.push(Number(n & 0x7fn) | 0x80); n >>= 7n; } out.push(Number(n)); }
function rv(b, p) {
  let r = 0n, s = 0n;
  for (;;) {
    if (p.i >= b.length) throw new Error('svp: truncated varint');
    const x = b[p.i++];
    r |= BigInt(x & 0x7f) << s;
    if (!(x & 0x80)) break;
    s += 7n;
    if (s > 126n) throw new Error('svp: varint overflow');
  }
  return r;
}
function rn(b, p) { const v = rv(b, p); if (v > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('svp: int too big'); return Number(v); }
function wstr(out, s) { const e = te.encode(s); wv(out, e.length); for (const x of e) out.push(x); }
function rstr(b, p) { const n = rn(b, p); if (p.i + n > b.length) throw new Error('svp: truncated str'); const s = td.decode(b.subarray(p.i, p.i + n)); p.i += n; return s; }

/** Canonical decimal -> [header, mantissa] or null. */
export function numPack(s) {
  if (!NUM_RE.test(s)) return null;
  const neg = s[0] === '-' ? 1 : 0;
  const body = neg ? s.slice(1) : s;
  const dot = body.indexOf('.');
  const frac = dot < 0 ? 0 : body.length - dot - 1;
  const digits = body.replace('.', '');
  if (digits.length > 30) return null;
  return [BigInt(frac * 2 + neg), BigInt(digits)];
}
export function numUnpack(h, m) {
  const neg = Number(h & 1n), frac = Number(h >> 1n);
  if (frac > 64) throw new Error('svp: frac too long');
  let d = m.toString().padStart(frac + 1, '0');
  if (frac) d = d.slice(0, d.length - frac) + '.' + d.slice(d.length - frac);
  return (neg ? '-' : '') + d;
}
function wnum(out, s) { const [h, m] = numPack(s); wv(out, h); wv(out, m); }
function rnum(b, p) { const h = rv(b, p); const m = rv(b, p); return numUnpack(h, m); }

function encVars(out, vars) {
  wv(out, vars.length);
  for (const v of vars) {
    wstr(out, v.name);
    const k = v.kind;
    if (k === 'rgb') { out.push(2); continue; }
    if (k.options) { out.push(0); wv(out, k.options.length); for (const o of k.options) wstr(out, o); continue; }
    if (k.range) {
      const { min, max, precision } = k.range;
      if (numPack(min) && numPack(max)) { out.push(1); wv(out, precision); wnum(out, min); wnum(out, max); }
      else { out.push(4); wv(out, precision); wstr(out, min); wstr(out, max); }
      continue;
    }
    if (k.rgb_styled) {
      out.push(3); wv(out, k.rgb_styled.length);
      for (const r of k.rgb_styled) out.push(r.r_min, r.r_max, r.g_min, r.g_max, r.b_min, r.b_max);
      continue;
    }
    throw new Error('svp: unknown variable kind for ' + v.name);
  }
}
function decVars(b, p) {
  const n = rn(b, p); const vars = [];
  for (let i = 0; i < n; i++) {
    const name = rstr(b, p);
    const t = b[p.i++];
    let kind;
    if (t === 0) { const c = rn(b, p); const o = []; for (let j = 0; j < c; j++) o.push(rstr(b, p)); kind = { options: o }; }
    else if (t === 1) { const precision = rn(b, p); kind = { range: { min: rnum(b, p), max: rnum(b, p), precision } }; }
    else if (t === 2) kind = 'rgb';
    else if (t === 3) {
      const c = rn(b, p); const r = [];
      for (let j = 0; j < c; j++) { if (p.i + 6 > b.length) throw new Error('svp: truncated rgb'); const [a, bb, c2, d, e, f] = b.subarray(p.i, p.i + 6); p.i += 6; r.push({ r_min: a, r_max: bb, g_min: c2, g_max: d, b_min: e, b_max: f }); }
      kind = { rgb_styled: r };
    } else if (t === 4) { const precision = rn(b, p); kind = { range: { min: rstr(b, p), max: rstr(b, p), precision } }; }
    else throw new Error('svp: bad var kind ' + t);
    vars.push({ name, kind });
  }
  return vars;
}

const isDig = (x) => x >= 0x30 && x <= 0x39;
function numAt(src, i) {
  let j = i;
  if (src[j] === 0x2d) j++;
  if (!(j < src.length && isDig(src[j]))) return 0;
  while (j < src.length && isDig(src[j])) j++;
  if (j + 1 < src.length && src[j] === 0x2e && isDig(src[j + 1])) { j++; while (j < src.length && isDig(src[j])) j++; }
  return j - i;
}

/** Encode { svg_template, variables } -> Uint8Array. */
export function packTemplate(template, variables) {
  const src = te.encode(template);
  const out = [...MAGIC];
  encVars(out, variables);
  const names = new Map(variables.map((v, i) => [v.name, i]).reverse());
  let lit = [];
  const flush = () => { if (lit.length) { out.push(OP.LIT); wv(out, lit.length); for (const x of lit) out.push(x); lit = []; } };
  let i = 0; const n = src.length;
  while (i < n) {
    if (src[i] === 0x24 && src[i + 1] === 0x7b) { // ${
      let j = i + 2; while (j < n && src[j] !== 0x7d) j++;
      if (j >= n) throw new Error('svp: unclosed placeholder at ' + i);
      const name = td.decode(src.subarray(i + 2, j));
      if (!names.has(name)) throw new Error(`svp: undefined variable '${name}'`);
      flush(); out.push(OP.VAR); wv(out, names.get(name)); i = j + 1; continue;
    }
    let best = null, bestSave = 1; // need saving >= 2
    // NL
    if (src[i] === 0x0a) {
      let s = 0; while (i + 1 + s < n && src[i + 1 + s] === 0x20) s++;
      const sv = 1 + s - (1 + vlen(s));
      if (sv > bestSave) { bestSave = sv; best = { op: OP.NL, len: 1 + s, a: s }; }
    }
    // NUM
    const nl = numAt(src, i);
    if (nl > 0) {
      const s = td.decode(src.subarray(i, i + nl)); const pk = numPack(s);
      if (pk) { const sv = nl - (1 + vlen(pk[0]) + vlen(pk[1])); if (sv > bestSave) { bestSave = sv; best = { op: OP.NUM, len: nl, a: pk[0], b: pk[1] }; } }
    }
    // DICT
    for (let d = 0; d < DICT_B.length; d++) {
      const w = DICT_B[d]; if (w.length > n - i) continue;
      let ok = true; for (let k = 0; k < w.length; k++) if (src[i + k] !== w[k]) { ok = false; break; }
      if (!ok) continue;
      const sv = w.length - 2;
      if (sv > bestSave) { bestSave = sv; best = { op: OP.DICT, len: w.length, a: d }; }
    }
    // COPY
    let cl = 0, cd = 0;
    for (let j = i - 1; j >= 0; j--) {
      let l = 0; while (i + l < n && src[j + l] === src[i + l]) l++;
      if (l > cl) { cl = l; cd = i - j; }
    }
    if (cl >= 4) {
      const sv = cl - (1 + vlen(cd) + vlen(cl - 4));
      if (sv > bestSave) { bestSave = sv; best = { op: OP.COPY, len: cl, a: cd, b: cl - 4 }; }
    }
    if (!best) { lit.push(src[i]); i++; continue; }
    flush(); out.push(best.op);
    if (best.op === OP.NL) wv(out, best.a);
    else if (best.op === OP.DICT) out.push(best.a);
    else { wv(out, best.a); wv(out, best.b); }
    i += best.len;
  }
  flush(); out.push(OP.END);
  return Uint8Array.from(out);
}

/** Decode -> { svg_template, variables }. Deterministic; bounded by MAX_OUT. */
export function unpackTemplate(bytes) {
  const b = bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes);
  for (let k = 0; k < 4; k++) if (b[k] !== MAGIC[k]) throw new Error('svp: bad magic');
  const p = { i: 4 };
  const variables = decVars(b, p);
  const out = [];
  const push = (x) => { if (out.length >= MAX_OUT) throw new Error('svp: output too large'); out.push(x); };
  for (;;) {
    if (p.i >= b.length) throw new Error('svp: missing END');
    const op = b[p.i++];
    if (op === OP.END) break;
    if (op === OP.LIT) { const l = rn(b, p); if (p.i + l > b.length) throw new Error('svp: truncated lit'); for (let k = 0; k < l; k++) push(b[p.i + k]); p.i += l; }
    else if (op === OP.DICT) { const w = DICT_B[b[p.i++]]; if (!w) throw new Error('svp: bad dict idx'); for (const x of w) push(x); }
    else if (op === OP.COPY) { const d = rn(b, p); const l = rn(b, p) + 4; if (d < 1 || d > out.length) throw new Error('svp: bad copy'); if (l > MAX_OUT) throw new Error('svp: output too large'); const s = out.length - d; for (let k = 0; k < l; k++) push(out[s + k]); }
    else if (op === OP.VAR) { const v = variables[rn(b, p)]; if (!v) throw new Error('svp: bad var idx'); for (const x of te.encode('${' + v.name + '}')) push(x); }
    else if (op === OP.NUM) { for (const x of te.encode(rnum(b, p))) push(x); }
    else if (op === OP.NL) { const s = rn(b, p); push(0x0a); for (let k = 0; k < s; k++) push(0x20); }
    else throw new Error('svp: bad op ' + op);
  }
  if (p.i !== b.length) throw new Error('svp: trailing bytes');
  return { svg_template: td.decode(Uint8Array.from(out)), variables };
}

/** Optional big-integer view: 16-byte little-endian u128 limbs (decimal strings). */
export function toLimbs(bytes) {
  const l = [];
  for (let i = 0; i < bytes.length; i += 16) { let v = 0n; for (let k = Math.min(15, bytes.length - 1 - i); k >= 0; k--) v = (v << 8n) | BigInt(bytes[i + k]); l.push(v.toString()); }
  return { len: bytes.length, limbs: l };
}
