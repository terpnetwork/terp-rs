// Minimal crypto for the Headstash claim page. No dependencies.
// sha256/hkdf/pbkdf2/aes-gcm via WebCrypto; secp256k1 (RFC6979), keccak256, ripemd160, bech32/bech32m, protobuf here.
export const te = new TextEncoder();
export const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
export const unhex = (h) => { h = h.replace(/^0x/, ""); if (h.length % 2 || /[^0-9a-f]/i.test(h)) throw new Error("bad hex"); const o = new Uint8Array(h.length / 2); for (let i = 0; i < o.length; i++) o[i] = parseInt(h.substr(i * 2, 2), 16); return o; };
export const cat = (...a) => { const o = new Uint8Array(a.reduce((n, x) => n + x.length, 0)); let p = 0; for (const x of a) { o.set(x, p); p += x.length; } return o; };
export const b64 = (b) => { let s = ""; for (const x of b) s += String.fromCharCode(x); return btoa(s); };
export const unb64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
export const u16le = (n) => new Uint8Array([n & 255, (n >> 8) & 255]);
export const u32le = (n) => new Uint8Array([n & 255, (n >> 8) & 255, (n >> 16) & 255, (n >>> 24) & 255]);
export async function sha256(b) { return new Uint8Array(await crypto.subtle.digest("SHA-256", b)); }
export async function hkdf(ikm, salt, info, bytes) {
  const k = await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt, info }, k, bytes * 8));
}
const big = (b) => BigInt("0x" + (hex(b) || "0"));
const be32 = (n) => unhex(n.toString(16).padStart(64, "0"));

// ---- keccak256 (BigInt lanes; inputs here are small) ----
const RC = ["1","8082","800000000000808a","8000000080008000","808b","80000001","8000000080008081","8000000000008009","8a","88","80008009","8000000a","8000808b","800000000000008b","8000000000008089","8000000000008003","8000000000008002","8000000000000080","800a","800000008000000a","8000000080008081","8000000000008080","80000001","8000000080008008"].map((x) => BigInt("0x" + x));
const ROT = [0,1,62,28,27,36,44,6,55,20,3,10,43,25,39,41,45,15,21,8,18,2,61,56,14];
const M64 = (1n << 64n) - 1n;
const rotl = (x, n) => (n === 0 ? x : ((x << BigInt(n)) | (x >> BigInt(64 - n))) & M64);
function keccakF(s) {
  for (let r = 0; r < 24; r++) {
    const C = [0, 1, 2, 3, 4].map((x) => s[x] ^ s[x + 5] ^ s[x + 10] ^ s[x + 15] ^ s[x + 20]);
    for (let x = 0; x < 5; x++) { const d = C[(x + 4) % 5] ^ rotl(C[(x + 1) % 5], 1); for (let y = 0; y < 25; y += 5) s[x + y] ^= d; }
    const B = new Array(25);
    for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) B[y + 5 * ((2 * x + 3 * y) % 5)] = rotl(s[x + 5 * y], ROT[x + 5 * y]);
    for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) s[x + 5 * y] = B[x + 5 * y] ^ (~B[(x + 1) % 5 + 5 * y] & M64 & B[(x + 2) % 5 + 5 * y]);
    s[0] ^= RC[r];
  }
}
export function keccak256(msg) {
  const rate = 136, s = new Array(25).fill(0n);
  const p = new Uint8Array(Math.ceil((msg.length + 1) / rate) * rate); p.set(msg); p[msg.length] ^= 1; p[p.length - 1] ^= 0x80;
  for (let o = 0; o < p.length; o += rate) {
    for (let i = 0; i < rate / 8; i++) { let v = 0n; for (let j = 7; j >= 0; j--) v = (v << 8n) | BigInt(p[o + i * 8 + j]); s[i] ^= v; }
    keccakF(s);
  }
  const out = new Uint8Array(32);
  for (let i = 0; i < 4; i++) for (let j = 0; j < 8; j++) out[i * 8 + j] = Number((s[i] >> BigInt(8 * j)) & 255n);
  return out;
}

// ---- ripemd160 ----
export function ripemd160(msg) {
  const rl = [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,7,4,13,1,10,6,15,3,12,0,9,5,2,14,11,8,3,10,14,4,9,15,8,1,2,7,0,6,13,11,5,12,1,9,11,10,0,8,12,4,13,3,7,15,14,5,6,2,4,0,5,9,7,12,2,10,14,1,3,8,11,6,15,13];
  const rr = [5,14,7,0,9,2,11,4,13,6,15,8,1,10,3,12,6,11,3,7,0,13,5,10,14,15,8,12,4,9,1,2,15,5,1,3,7,14,6,9,11,8,12,2,10,0,4,13,8,6,4,1,3,11,15,0,5,12,2,13,9,7,10,14,12,15,10,4,1,5,8,7,6,2,13,14,0,3,9,11];
  const sl = [11,14,15,12,5,8,7,9,11,13,14,15,6,7,9,8,7,6,8,13,11,9,7,15,7,12,15,9,11,7,13,12,11,13,6,7,14,9,13,15,14,8,13,6,5,12,7,5,11,12,14,15,14,15,9,8,9,14,5,6,8,6,5,12,9,15,5,11,6,8,13,12,5,12,13,14,11,8,5,6];
  const sr = [8,9,9,11,13,15,15,5,7,7,8,11,14,14,12,6,9,13,15,7,12,8,9,11,7,7,12,7,6,15,13,11,9,7,15,11,8,6,6,14,12,13,5,14,13,13,7,5,15,5,8,11,14,14,6,14,6,9,12,9,12,5,15,8,8,5,12,9,12,5,14,6,8,13,6,5,15,13,11,11];
  const KL = [0, 0x5a827999, 0x6ed9eba1, 0x8f1bbcdc, 0xa953fd4e], KR = [0x50a28be6, 0x5c4dd124, 0x6d703ef3, 0x7a6d76e9, 0];
  const f = (j, x, y, z) => j < 16 ? x ^ y ^ z : j < 32 ? (x & y) | (~x & z) : j < 48 ? (x | ~y) ^ z : j < 64 ? (x & z) | (y & ~z) : x ^ (y | ~z);
  const rol = (x, n) => (x << n) | (x >>> (32 - n));
  const len = msg.length, pl = Math.ceil((len + 9) / 64) * 64, p = new Uint8Array(pl); p.set(msg); p[len] = 0x80;
  const bits = len * 8; for (let i = 0; i < 4; i++) p[pl - 8 + i] = (bits >>> (8 * i)) & 255;
  let h = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
  for (let o = 0; o < pl; o += 64) {
    const X = []; for (let i = 0; i < 16; i++) X.push(p[o + 4 * i] | (p[o + 4 * i + 1] << 8) | (p[o + 4 * i + 2] << 16) | (p[o + 4 * i + 3] << 24));
    let [al, bl, cl, dl, el] = h, [ar, br, cr, dr, er] = h;
    for (let j = 0; j < 80; j++) {
      let t = (rol((al + f(j, bl, cl, dl) + X[rl[j]] + KL[j >> 4]) | 0, sl[j]) + el) | 0; al = el; el = dl; dl = rol(cl, 10); cl = bl; bl = t;
      t = (rol((ar + f(79 - j, br, cr, dr) + X[rr[j]] + KR[j >> 4]) | 0, sr[j]) + er) | 0; ar = er; er = dr; dr = rol(cr, 10); cr = br; br = t;
    }
    const t = (h[1] + cl + dr) | 0; h[1] = (h[2] + dl + er) | 0; h[2] = (h[3] + el + ar) | 0; h[3] = (h[4] + al + br) | 0; h[4] = (h[0] + bl + cr) | 0; h[0] = t;
  }
  const out = new Uint8Array(20); h.forEach((w, i) => { for (let j = 0; j < 4; j++) out[4 * i + j] = (w >>> (8 * j)) & 255; }); return out;
}

// ---- bech32 / bech32m ----
const CS = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
function polymod(v) { const G = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]; let c = 1; for (const x of v) { const b = c >> 25; c = ((c & 0x1ffffff) << 5) ^ x; for (let i = 0; i < 5; i++) if ((b >> i) & 1) c ^= G[i]; } return c; }
const hrpx = (h) => [...[...h].map((c) => c.charCodeAt(0) >> 5), 0, ...[...h].map((c) => c.charCodeAt(0) & 31)];
function conv(data, from, to, pad) { let acc = 0, bits = 0; const o = [], m = (1 << to) - 1; for (const v of data) { acc = (acc << from) | v; bits += from; while (bits >= to) { bits -= to; o.push((acc >> bits) & m); } } if (pad && bits) o.push((acc << (to - bits)) & m); else if (!pad && (bits >= from || ((acc << (to - bits)) & m))) throw new Error("bad padding"); return o; }
export function bech32Encode(hrp, bytes) { const d = conv(bytes, 8, 5, true); const pm = polymod([...hrpx(hrp), ...d, 0, 0, 0, 0, 0, 0]) ^ 1; const c = [0, 1, 2, 3, 4, 5].map((i) => (pm >> (5 * (5 - i))) & 31); return hrp + "1" + [...d, ...c].map((x) => CS[x]).join(""); }
export function bech32Decode(s, m = false) {
  if (s !== s.toLowerCase()) throw new Error("mixed case"); const p = s.lastIndexOf("1"); if (p < 1) throw new Error("no separator");
  const hrp = s.slice(0, p), d = [...s.slice(p + 1)].map((c) => { const i = CS.indexOf(c); if (i < 0) throw new Error("bad char"); return i; });
  if (d.length < 6 || polymod([...hrpx(hrp), ...d]) !== (m ? 0x2bc830a3 : 1)) throw new Error("bad checksum");
  return { hrp, bytes: new Uint8Array(conv(d.slice(0, -6), 5, 8, false)) };
}

// ---- secp256k1 ----
const P = 2n ** 256n - 2n ** 32n - 977n, N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
const G = [0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798n, 0x483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8n];
const md = (a, m = P) => ((a % m) + m) % m;
function inv(a, m = P) { let [x, y, u, v] = [md(a, m), m, 1n, 0n]; while (x) { const q = y / x; [x, y] = [y - q * x, x]; [u, v] = [v - q * u, u]; } return md(v, m); }
// Jacobian
function jdbl([X, Y, Z]) { if (!Y) return [0n, 1n, 0n]; const S = md(4n * X * Y * Y), M = md(3n * X * X), X2 = md(M * M - 2n * S); return [X2, md(M * (S - X2) - 8n * Y ** 4n), md(2n * Y * Z)]; }
function jadd(a, b) {
  if (!a[2]) return b; if (!b[2]) return a;
  const Z1Z1 = md(a[2] * a[2]), Z2Z2 = md(b[2] * b[2]), U1 = md(a[0] * Z2Z2), U2 = md(b[0] * Z1Z1), S1 = md(a[1] * b[2] * Z2Z2), S2 = md(b[1] * a[2] * Z1Z1);
  if (U1 === U2) return S1 === S2 ? jdbl(a) : [0n, 1n, 0n];
  const H = md(U2 - U1), R = md(S2 - S1), H2 = md(H * H), H3 = md(H2 * H), X3 = md(R * R - H3 - 2n * U1 * H2);
  return [X3, md(R * (U1 * H2 - X3) - S1 * H3), md(H * a[2] * b[2])];
}
function mul(k, pt = G) { let r = [0n, 1n, 0n], q = [pt[0], pt[1], 1n]; while (k > 0n) { if (k & 1n) r = jadd(r, q); q = jdbl(q); k >>= 1n; } const zi = inv(r[2]); return [md(r[0] * zi * zi), md(r[1] * zi * zi * zi)]; }
export function privOk(d) { const k = big(d); return d.length === 32 && k > 0n && k < N; }
export function pubkey(d) { const [x, y] = mul(big(d)); return { x, y, compressed: cat(new Uint8Array([y & 1n ? 3 : 2]), be32(x)), uncompressed: cat(be32(x), be32(y)) }; }
async function hmac(key, data) { const k = await crypto.subtle.importKey("raw", key, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]); return new Uint8Array(await crypto.subtle.sign("HMAC", k, data)); }
async function rfc6979(d, h) {
  let V = new Uint8Array(32).fill(1), K = new Uint8Array(32); const x = be32(big(d)), hm = be32(md(big(h), N));
  K = await hmac(K, cat(V, [0], x, hm)); V = await hmac(K, V); K = await hmac(K, cat(V, [1], x, hm)); V = await hmac(K, V);
  for (;;) { V = await hmac(K, V); const k = big(V); if (k > 0n && k < N) return k; K = await hmac(K, cat(V, [0])); V = await hmac(K, V); }
}
// returns {r,s,rec} with low-s normalization
export async function sign(d, digest) {
  const k = await rfc6979(d, digest), R = mul(k), r = md(R[0], N); let s = md(inv(k, N) * (md(big(digest), N) + r * big(d)), N), rec = Number(R[1] & 1n) | (R[0] >= N ? 2 : 0);
  if (s > N / 2n) { s = N - s; rec ^= 1; }
  return { r: be32(r), s: be32(s), rec };
}
export function reduceN(b) { return be32(md(big(b), N - 1n) + 1n); }

// ---- protobuf (just what cosmos tx needs) ----
const varint = (n) => { n = BigInt(n); const o = []; while (n > 127n) { o.push(Number(n & 127n) | 128); n >>= 7n; } o.push(Number(n)); return new Uint8Array(o); };
export const pb = {
  bytes: (f, b) => (b && b.length ? cat(varint((f << 3) | 2), varint(b.length), b) : new Uint8Array()),
  str: (f, s) => pb.bytes(f, te.encode(s || "")),
  uint: (f, n) => (BigInt(n) ? cat(varint(f << 3), varint(n)) : new Uint8Array()),
  any: (f, type, val) => pb.bytes(f, cat(pb.str(1, type), pb.bytes(2, val))),
};
