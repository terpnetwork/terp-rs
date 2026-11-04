// node --test tools/headstash/frontend/test/   (Node >= 20; no dependencies)
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as V from "../v2lib.js";
import { bech32Decode } from "../crypto.js";
const here = (p) => new URL("../" + p, import.meta.url);
const te = new TextEncoder();

// Same vectors as cw-headstash src/forward.rs `frontend_vectors` (cross-checked in Rust).
const FWD = [
  [["terp-local-1", "terp1contract", { channel_id: "channel-7", receiver: "penumbra1xyz", memo: null, fallback: "terp1fallback" }], "hsfwd1l3oJE1L5SZTc5Dc42Mim0GCH3GE4JmKom3DyMt0OmjM"],
  [["90u-4", "terp1abc", { channel_id: "channel-0", receiver: "osmo1r", memo: "hello", fallback: "terp1fb" }], "hsfwd1F_KbPO0Hl2dNEgw3H3QyJAoYhxxqFjsMnHsvN6XmaRY"],
];
test("forward destination matches forward.rs vectors", async () => {
  for (const [[c, k, t], want] of FWD) { const d = await V.forwardDestination(c, k, t); assert.equal(d, want); assert.equal(d.length, 49); }
});
test("forward destination: memo null == empty, every field binds", async () => {
  const t = FWD[0][0][2], base = await V.forwardDestination("a", "b", t);
  assert.equal(await V.forwardDestination("a", "b", { ...t, memo: "" }), base);
  for (const k of ["channel_id", "receiver", "fallback"]) assert.notEqual(await V.forwardDestination("a", "b", { ...t, [k]: t[k] + "x" }), base);
  assert.notEqual(await V.forwardDestination("a2", "b", t), base); assert.notEqual(await V.forwardDestination("a", "b2", t), base);
  // length-prefixing: moving a byte between fields changes the hash
  assert.notEqual(await V.forwardDestination("ab", "", t), await V.forwardDestination("a", "b", t));
});
test("channel / receiver validation mirrors the contract", () => {
  assert.ok(V.validChannel("channel-0")); assert.ok(V.validChannel("channel-1234567890"));
  for (const c of ["channel-", "channel-12345678901", "chan-1", "channel-1a", " channel-1"]) assert.ok(!V.validChannel(c), c);
  assert.ok(V.validReceiver("penumbra1abc")); assert.ok(!V.validReceiver("")); assert.ok(!V.validReceiver("a b")); assert.ok(!V.validReceiver("x".repeat(257))); assert.ok(V.validReceiver("x".repeat(256)));
});

const bytes = te.encode(JSON.stringify({ v: 1, kind: "terp-headstash-hal", eligibility_root: "ab".repeat(32), depth: 2, rows: [{ identity: "0xAA", note_index: 1, leaf_index: 5, path: ["00", "11"], class: 0 }, { identity: "aa", note_index: 0, leaf_index: 4, path: ["00", "11"], class: 0 }] }));
test("HAL: sha256 pin verifies, tamper refused", async () => {
  const h = V.hexOf(await V.sha256(bytes));
  assert.deepEqual((await V.verifyHal(bytes, { uri: "x", sha256: h })).passed, ["sha256"]);
  const bad = bytes.slice(); bad[10] ^= 1;
  await assert.rejects(V.verifyHal(bad, { uri: "x", sha256: h }), /sha256 mismatch/);
  await assert.rejects(V.verifyHal(bytes, { uri: "x", sha256: "zz" }), /not 32-byte hex/);
});
test("HAL: raw CIDv1 verifies; dag-pb alone refused; nothing pinned refused", async () => {
  const d = await V.sha256(bytes), cid = V.rawCid(d);
  assert.match(cid, /^bafkrei[a-z2-7]{52}$/);
  assert.deepEqual((await V.verifyHal(bytes, { uri: "x", cid })).passed, ["cid-raw"]);
  assert.deepEqual((await V.verifyHal(bytes, { uri: "x", cid, sha256: V.hexOf(d) })).passed, ["sha256", "cid-raw"]);
  const other = V.rawCid(new Uint8Array(32));
  await assert.rejects(V.verifyHal(bytes, { uri: "x", cid: other }), /CID .*mismatch/);
  await assert.rejects(V.verifyHal(bytes, { uri: "x", cid: "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG" }), /dag-pb/);
  await assert.rejects(V.verifyHal(bytes, { uri: "x" }), /neither/);
  // known external vector: raw CID of empty bytes
  assert.equal(V.rawCid(await V.sha256(new Uint8Array())), "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku");
});
test("HAL: parse binds to the chain root and indexes identities", () => {
  const p = V.parseHal(bytes, "AB".repeat(32));
  assert.equal(p.byIdentity.get("aa").length, 2); assert.equal(p.byIdentity.get("aa")[0].note_index, 0);
  assert.throws(() => V.parseHal(bytes, "cd".repeat(32)), /another eligibility root/);
  assert.throws(() => V.parseHal(te.encode("{}"), null), /unknown HAL format/);
  assert.throws(() => V.parseHal(te.encode("nope"), null), /not JSON/);
});
test("HAL uri resolution", () => {
  const g = ["https://g1", "https://g2"];
  assert.deepEqual(V.resolveHal({ uri: "https://s3.terp.network/hal.json" }, g), ["https://s3.terp.network/hal.json"]);
  assert.deepEqual(V.resolveHal({ uri: "s3.terp.network/hal.json" }, g), ["https://s3.terp.network/hal.json"]);
  assert.deepEqual(V.resolveHal({ uri: "127.0.0.1:3921/hal.json" }, g), ["http://127.0.0.1:3921/hal.json"]);
  assert.deepEqual(V.resolveHal({ uri: "ipfs://bafy/hal.json" }, g), ["https://g1/ipfs/bafy/hal.json", "https://g2/ipfs/bafy/hal.json"]);
  assert.deepEqual(V.resolveHal({ uri: "ar://tx1" }, g), ["https://arweave.net/tx1"]);
  assert.deepEqual(V.resolveHal({ uri: "h.example/x", cid: "bafk" }, g), ["https://h.example/x", "https://g1/ipfs/bafk", "https://g2/ipfs/bafk"]);
  assert.throws(() => V.resolveHal({ uri: "http://evil/x" }, g), /unsupported/);
  assert.throws(() => V.resolveHal(null, g), /no HAL/);
});
test("reuse guard: one owner per address, persisted, case-insensitive", async () => {
  const s = new V.MemStorage(), g = new V.ReuseGuard(s);
  assert.ok(await g.use("terp1abc", "claim", "nf1"));
  assert.ok(await g.use("terp1abc", "claim", "nf1")); // same note retry is fine
  await assert.rejects(g.use("TERP1ABC", "claim", "nf2"), V.ReuseError);
  await assert.rejects(new V.ReuseGuard(s).check("terp1abc", "nf3"), /already used/);
  assert.equal(await new V.ReuseGuard(s).check("terp1new", "nf3"), true);
  assert.equal(g.count(), 1); assert.ok(!s.getItem(V.GUARD_KEY).includes("terp1abc"), "addresses stored hashed only");
});
test("penumbra address check (bech32m, 80 bytes)", () => {
  const a = V.bech32mEncode("penumbra", new Uint8Array(80).fill(7));
  assert.equal(V.checkPenumbra(a, bech32Decode), a);
  assert.throws(() => V.checkPenumbra(V.bech32mEncode("penumbra", new Uint8Array(32)), bech32Decode), /80 bytes/);
  assert.throws(() => V.checkPenumbra(V.bech32mEncode("cosmos", new Uint8Array(80)), bech32Decode), /hrp/);
  assert.throws(() => V.checkPenumbra(a.slice(0, -1) + (a.endsWith("q") ? "p" : "q"), bech32Decode), /checksum/);
});
test("every form step has an info modal entry and button", () => {
  const html = readFileSync(here("app.html"), "utf8"), steps = JSON.parse(readFileSync(here("steps.json"), "utf8"));
  const ids = [...new Set([...html.matchAll(/data-info="([a-z-]+)"/g)].map((m) => m[1]))];
  assert.ok(ids.length >= 10); assert.deepEqual(ids.sort(), Object.keys(steps).sort()); assert.deepEqual(V.missingSteps(html, steps), []);
  assert.deepEqual(V.missingSteps(html.replace('data-info="hal"', ""), steps), ["hal"]);
  const s2 = { ...steps }; delete s2.claim; assert.deepEqual(V.missingSteps(html, s2), ["claim"]);
});
test("code id list parsing", () => {
  assert.deepEqual(V.parseNumList("12, 14 15"), ["12", "14", "15"]); assert.throws(() => V.parseNumList("12,x"), /number/);
  assert.equal(V.codeContractsPath(3, "a+b"), "/cosmwasm/wasm/v1/code/3/contracts?pagination.limit=100&pagination.key=a%2Bb");
});

// ---- Zafu index allocation + address validation ----
test("IndexBook: fresh 12-byte non-zero randomizers, exact {account, randomizer} shape", () => {
  const b = new V.IndexBook(new V.MemStorage()), seen = new Set();
  for (let i = 0; i < 200; i++) {
    const x = b.allocate(i % 3); assert.deepEqual(Object.keys(x).sort(), ["account", "randomizer"]);
    assert.equal(x.account, i % 3); assert.ok(x.randomizer instanceof Uint8Array); assert.equal(x.randomizer.length, 12);
    assert.ok(x.randomizer.some((v) => v)); const k = V.IndexBook.key(x.account, x.randomizer); assert.ok(!seen.has(k)); seen.add(k);
    b.record(x, "penumbra1addr" + i, "n" + i);
  }
  assert.equal(b.count(), 200);
});
test("IndexBook: zero randomizer and used indices are skipped; broken rng fails closed", () => {
  const st = new V.MemStorage(), fixed = Uint8Array.from({ length: 12 }, (_, i) => i + 1);
  const seq = [new Uint8Array(12), fixed, fixed, Uint8Array.from(fixed, (v) => v ^ 0xff)]; let n = 0;
  const b = new V.IndexBook(st, (buf) => { buf.set(seq[n++ % seq.length]); return buf; });
  const a = b.allocate(0); assert.deepEqual([...a.randomizer], [...fixed]); b.record(a, "penumbra1a", "x");
  const c = b.allocate(0); assert.deepEqual([...c.randomizer], [...seq[3]]);
  const z = new V.IndexBook(st, (buf) => buf.fill(0)); assert.throws(() => z.allocate(0), /fresh address index/);
  assert.throws(() => new V.IndexBook(st, () => new Uint8Array(11)).allocate(0), /12 bytes/);
});
test("IndexBook: record refuses index or address reuse, and zero/malformed randomizers", () => {
  const b = new V.IndexBook(new V.MemStorage()), r = Uint8Array.from({ length: 12 }, () => 7);
  b.record({ account: 1, randomizer: r }, "penumbra1aaa", "o1");
  assert.throws(() => b.record({ account: 1, randomizer: r }, "penumbra1bbb", "o2"), V.ReuseError);
  assert.throws(() => b.record({ account: 2, randomizer: r }, "PENUMBRA1AAA ", "o2"), V.ReuseError);
  b.record({ account: 2, randomizer: r }, "penumbra1ccc", "o3"); // same randomizer, other account = other index
  assert.throws(() => b.record({ account: 0, randomizer: new Uint8Array(12) }, "penumbra1d", "o"), /zero/);
  assert.throws(() => b.record({ account: 0, randomizer: new Uint8Array(16).fill(1) }, "penumbra1e", "o"), /malformed/);
  assert.throws(() => b.record({ account: 0, randomizer: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] }, "penumbra1f", "o"), /malformed/);
});
test("parseAccount: u32 only", () => {
  assert.equal(V.parseAccount("0"), 0); assert.equal(V.parseAccount(" 4294967295 "), 4294967295);
  for (const s of ["-1", "1.5", "4294967296", "", "0x1", "abc"]) assert.throws(() => V.parseAccount(s), s);
});
test("Penumbra address validation: bech32m, hrp penumbra, exactly 80 bytes", () => {
  const ok = V.bech32mEncode("penumbra", Uint8Array.from({ length: 80 }, (_, i) => i));
  assert.equal(V.checkPenumbra(ok, bech32Decode), ok); assert.ok(ok.startsWith("penumbra1")); assert.equal(ok.length, 143);
  assert.throws(() => V.checkPenumbra(V.bech32mEncode("penumbra", new Uint8Array(79)), bech32Decode), /80/);
  assert.throws(() => V.checkPenumbra(V.bech32mEncode("penumbra", new Uint8Array(81)), bech32Decode), /80/);
  assert.throws(() => V.checkPenumbra(V.bech32mEncode("penumbrafullviewingkey", new Uint8Array(80)), bech32Decode));
  const bad = ok.slice(0, -1) + (ok.endsWith("q") ? "p" : "q"); assert.throws(() => V.checkPenumbra(bad, bech32Decode));
});
test("zafuErrorText maps provider failures", () => {
  assert.match(V.zafuErrorText(new Error("Connection request failed", { cause: "Denied" })), /declined/);
  assert.match(V.zafuErrorText(new Error("x", { cause: new Error("Connection request failed", { cause: "NeedsLogin" }) })), /locked/);
  assert.match(V.zafuErrorText(new Error("[unknown] penumbra network not enabled")), /enable and select/);
});

// ---- zafu-client.js (bundled @penumbra-zone libs): addressAtIndex proof checks against a fake ViewService ----
test("zafu-client addressAtIndex: accepts only a round-tripping address whose IndexByAddress matches", async () => {
  const Z = await import("../zafu-client.js"), inner = Uint8Array.from({ length: 80 }, (_, i) => (i * 7) & 255), r = Uint8Array.from({ length: 12 }, (_, i) => i + 1);
  const mk = (o = {}) => ({ addressByIndex: async (q) => { assert.equal(q.addressIndex.account, 3); assert.deepEqual([...q.addressIndex.randomizer], [...r]); return { address: { inner: o.inner || inner } }; },
    indexByAddress: async () => (o.noidx ? {} : { addressIndex: { account: o.acct ?? 3, randomizer: o.r || r } }) });
  const a = await Z.addressAtIndex(mk(), { account: 3, randomizer: r });
  assert.equal(a.address, V.bech32mEncode("penumbra", inner)); V.checkPenumbra(a.address, bech32Decode);
  assert.deepEqual([...Z.decodeAddress(a.address)], [...inner]);
  await assert.rejects(Z.addressAtIndex(mk({ inner: new Uint8Array(79) }), { account: 3, randomizer: r }), /malformed/);
  await assert.rejects(Z.addressAtIndex(mk({ noidx: true }), { account: 3, randomizer: r }), /does not recognise/);
  await assert.rejects(Z.addressAtIndex(mk({ acct: 4 }), { account: 3, randomizer: r }), /different index/);
  await assert.rejects(Z.addressAtIndex(mk({ r: new Uint8Array(12) }), { account: 3, randomizer: r }), /different index/);
  await assert.rejects(Z.addressAtIndex(mk(), { account: 3, randomizer: new Uint8Array(16) }), /12 bytes/);
  assert.deepEqual(Z.providers({}), []);
  const g = { [Symbol.for("penumbra")]: { ["chrome-extension://" + "a".repeat(32)]: { connect() {} }, "https://evil": { connect() {} }, ["chrome-extension://" + "b".repeat(32)]: {} } };
  assert.deepEqual(Z.providers(g).map((p) => p.origin), ["chrome-extension://" + "a".repeat(32)]);
});
