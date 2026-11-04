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
  const ids = [...html.matchAll(/data-step="([a-z-]+)"/g)].map((m) => m[1]);
  assert.ok(ids.length >= 9); assert.deepEqual(V.missingSteps(html, steps), []);
  assert.deepEqual(V.missingSteps(html.replace('data-info="hal"', ""), steps), ["hal"]);
  const s2 = { ...steps }; delete s2.claim; assert.deepEqual(V.missingSteps(html, s2), ["claim"]);
});
test("code id list parsing", () => {
  assert.deepEqual(V.parseNumList("12, 14 15"), ["12", "14", "15"]); assert.throws(() => V.parseNumList("12,x"), /number/);
  assert.equal(V.codeContractsPath(3, "a+b"), "/cosmwasm/wasm/v1/code/3/contracts?pagination.limit=100&pagination.key=a%2Bb");
});
