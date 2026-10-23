#!/usr/bin/env node
// JS dependency scraper: the full npm dependency tree (dependencies, optional,
// peer-resolved; devDependencies of workspace members only) of one or more
// front ends, read from package.json + lockfile (pnpm-lock.yaml v9 or
// package-lock.json v2/v3; no install needed), merged into one JSON graph in
// the rust-deps format (public/graphs/js-deps.html reads it).
//
//   node gen-js-deps.mjs [--out FILE] [--config roots.json] [--root id=PATH]...
//                        [--only id,id] [--no-dev]
//
// Defaults: --config is roots.json next to this file; --out is
// target/gen-deps/js-deps.json under the current directory. See README.md.
// Read-only. Output carries no filesystem paths.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const opt = { out: null, config: path.join(here, 'roots.json'), roots: [], only: null, dev: true };
for (let i = 0; i < argv.length; i++) {
  const a = argv[i], v = () => argv[++i];
  if (a === '--out' || a === '-o') opt.out = v();
  else if (a === '--config') opt.config = v();
  else if (a === '--root') opt.roots.push(v());
  else if (a === '--only') opt.only = v().split(',');
  else if (a === '--no-dev') opt.dev = false;
  else if (a === '-h' || a === '--help') { console.log(fs.readFileSync(fileURLToPath(import.meta.url), 'utf8').split('\nimport')[0]); process.exit(0); }
  else if (!a.startsWith('-') && !opt.out) opt.out = a;
  else { console.error(`unknown argument ${a}`); process.exit(2); }
}
const out = path.resolve(opt.out || 'target/gen-deps/js-deps.json');
const expand = (s) => String(s).replace(/\$\{(\w+)(?::-([^}]*))?\}/g, (_, k, d) => process.env[k] || d || '').replace(/^~(?=\/|$)/, os.homedir());
const cfgDir = path.dirname(path.resolve(opt.config));
let ROOTS = JSON.parse(fs.readFileSync(opt.config, 'utf8')).roots.map((r) => ({ ...r, path: expand(r.path) ? path.resolve(cfgDir, expand(r.path)) : '' }));
for (const spec of opt.roots) {
  const [id, p] = spec.split('='); const r = ROOTS.find((x) => x.id === id);
  if (r) r.path = path.resolve(expand(p)); else ROOTS.push({ id, label: id, path: path.resolve(expand(p)) });
}
if (opt.only) ROOTS = ROOTS.filter((r) => opt.only.includes(r.id));
const readJSON = (f) => JSON.parse(fs.readFileSync(f, 'utf8'));

// Minimal block-YAML reader for pnpm lockfiles: nested maps by indentation,
// scalars, flow values kept as raw strings, block sequences as arrays.
function parseYaml(text) {
  const root = {}; const stack = [{ ind: -1, obj: root }];
  const unq = (s) => { s = s.trim(); if (s.startsWith("'") && s.endsWith("'")) return s.slice(1, -1).replace(/''/g, "'"); if (s.startsWith('"') && s.endsWith('"')) return JSON.parse(s); return s; };
  for (const line of text.split('\n')) {
    if (!line.trim() || line.trimStart().startsWith('#')) continue;
    const ind = line.length - line.trimStart().length; const t = line.trim();
    while (stack.length > 1 && stack[stack.length - 1].ind >= ind) stack.pop();
    const top = stack[stack.length - 1];
    if (t.startsWith('- ')) { if (!Array.isArray(top.obj.__arr)) top.obj.__arr = []; top.obj.__arr.push(unq(t.slice(2))); continue; }
    let k, rest;
    if (t[0] === "'" || t[0] === '"') { const q = t[0]; let j = 1; for (; j < t.length; j++) { if (t[j] === q) { if (q === "'" && t[j + 1] === "'") { j++; continue; } break; } } k = unq(t.slice(0, j + 1)); rest = t.slice(j + 1).replace(/^:/, '').trim(); }
    else { const c = t.indexOf(': '); if (t.endsWith(':') && (c < 0 || c === t.length - 1)) { k = t.slice(0, -1); rest = ''; } else if (c < 0) continue; else { k = t.slice(0, c); rest = t.slice(c + 2).trim(); } }
    if (rest === '') { const o = {}; top.obj[k] = o; stack.push({ ind, obj: o }); }
    else top.obj[k] = rest === '{}' ? {} : unq(rest);
  }
  return root;
}
const stripPeers = (v) => String(v).replace(/\(.*$/, '');

// Each reader returns { lockfile, members: [{key,name}], pkgs: Map key→{n,v,s}, deps: Map key→[[toKey, kind]] }
// kind: 0 prod, 1 dev (members only), 2 optional.
function readPnpm(dir, file) {
  const y = parseYaml(fs.readFileSync(file, 'utf8'));
  const pkgs = new Map(); const deps = new Map(); const members = [];
  const pk = (name, ver) => { const v = stripPeers(ver); const key = `${name}@${v}`; if (!pkgs.has(key)) pkgs.set(key, { n: name, v, s: 'registry' }); return key; };
  const importers = y.importers || {};
  const impKey = (p) => `workspace:${p}`;
  for (const p of Object.keys(importers)) {
    let name = p; try { name = readJSON(path.join(dir, p, 'package.json')).name || p; } catch { /* no manifest */ }
    let ver = ''; try { ver = readJSON(path.join(dir, p, 'package.json')).version || ''; } catch { /* */ }
    pkgs.set(impKey(p), { n: name, v: ver, s: 'workspace' }); members.push(impKey(p));
  }
  for (const [p, imp] of Object.entries(importers)) {
    const list = [];
    for (const [sec, kind] of [['dependencies', 0], ['optionalDependencies', 2], ['devDependencies', 1]]) {
      if (kind === 1 && !opt.dev) continue;
      for (const [name, spec] of Object.entries(imp[sec] || {})) {
        const ver = typeof spec === 'object' ? spec.version : spec; if (!ver) continue;
        if (String(ver).startsWith('link:')) { const tgt = path.posix.normalize(path.posix.join(p === '.' ? '' : p, ver.slice(5))) || '.'; list.push([impKey(tgt), kind]); if (!pkgs.has(impKey(tgt))) { pkgs.set(impKey(tgt), { n: name, v: '', s: 'workspace' }); members.push(impKey(tgt)); } continue; }
        list.push([pk(name, ver), kind]);
      }
    }
    deps.set(impKey(p), list);
  }
  const snaps = y.snapshots || {};
  for (const [sk, s] of Object.entries(snaps)) {
    const base = stripPeers(sk); const at = base.lastIndexOf('@'); const key = pk(base.slice(0, at), base.slice(at + 1));
    const list = deps.get(key) || [];
    for (const [sec, kind] of [['dependencies', 0], ['optionalDependencies', 2]]) {
      for (const [name, ver] of Object.entries((s && s[sec]) || {})) { if (String(ver).startsWith('link:')) continue; const alias = String(ver).match(/^(@?[^@]+)@(.+)$/); list.push(alias && !/^\d/.test(ver) ? [pk(alias[1], alias[2]), kind] : [pk(name, ver), kind]); }
    }
    deps.set(key, list);
  }
  return { lockfile: `pnpm-lock.yaml v${String(y.lockfileVersion || '?').replace(/'/g, '')}`, members, pkgs, deps };
}
function readNpm(dir, file) {
  const lock = readJSON(file); const P = lock.packages || {};
  const pkgs = new Map(); const deps = new Map(); const members = [];
  const keyOf = (loc) => { const e = P[loc]; if (!e) return null; if (loc === '' || (e.link !== true && !loc.includes('node_modules/'))) return `workspace:${loc || '.'}`; if (e.link) return `workspace:${e.resolved}`; const name = e.name || loc.slice(loc.lastIndexOf('node_modules/') + 13); return `${name}@${e.version}`; };
  const resolve = (from, name) => { let base = from; for (;;) { const cand = (base ? base + '/' : '') + 'node_modules/' + name; if (P[cand]) return cand; if (!base) return null; const i = base.lastIndexOf('/node_modules/'); base = i < 0 ? '' : base.slice(0, i); } };
  for (const [loc, e] of Object.entries(P)) {
    const key = keyOf(loc); if (!key) continue;
    const isWs = key.startsWith('workspace:');
    if (!pkgs.has(key)) pkgs.set(key, isWs ? { n: e.name || (loc ? path.posix.basename(loc) : lock.name), v: e.version || '', s: 'workspace' } : { n: key.slice(0, key.lastIndexOf('@')), v: e.version, s: 'registry' });
    if (isWs && !members.includes(key)) members.push(key);
    if (e.link) continue;
    const list = deps.get(key) || [];
    for (const [sec, kind] of [['dependencies', 0], ['optionalDependencies', 2], ['peerDependencies', 0], ['devDependencies', 1]]) {
      if (kind === 1 && (!isWs || !opt.dev)) continue;
      for (const name of Object.keys(e[sec] || {})) { const r = resolve(loc, name); const k = r && keyOf(r); if (k) list.push([k, kind]); }
    }
    deps.set(key, list);
  }
  return { lockfile: `package-lock.json v${lock.lockfileVersion}`, members, pkgs, deps };
}
function readManifestOnly(dir) {
  const m = readJSON(path.join(dir, 'package.json')); const pkgs = new Map(); const deps = new Map(); const key = 'workspace:.';
  pkgs.set(key, { n: m.name || path.basename(dir), v: m.version || '', s: 'workspace' }); const list = [];
  for (const [sec, kind] of [['dependencies', 0], ['optionalDependencies', 2], ['devDependencies', 1]]) for (const [n, spec] of Object.entries(m[sec] || {})) { if (kind === 1 && !opt.dev) continue; const k = `${n}@${spec}`; pkgs.set(k, { n, v: spec, s: 'unlocked' }); list.push([k, kind]); }
  deps.set(key, list); return { lockfile: 'none (package.json only)', members: [key], pkgs, deps };
}

const nodesOut = []; const nodeIdx = new Map(); const edgeMap = new Map(); const roots = [];
const idxOf = (key0, p, rid) => { const key = key0.startsWith('workspace:') ? `${rid}/${key0}` : key0; if (!nodeIdx.has(key)) { nodeIdx.set(key, nodesOut.length); nodesOut.push({ n: p.n, v: p.v, s: p.s, r: {} }); } return nodeIdx.get(key); };
ROOTS.forEach((R, ri) => {
  const dir = R.path;
  if (!dir || !fs.existsSync(path.join(dir, 'package.json'))) { console.warn(`skip ${R.id}: no package.json`); return; }
  const g = fs.existsSync(path.join(dir, 'pnpm-lock.yaml')) ? readPnpm(dir, path.join(dir, 'pnpm-lock.yaml'))
    : fs.existsSync(path.join(dir, 'package-lock.json')) ? readNpm(dir, path.join(dir, 'package-lock.json')) : readManifestOnly(dir);
  let commit = ''; try { commit = execFileSync('git', ['log', '-1', '--format=%H', '--', '.'], { cwd: dir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim(); } catch { /* not git */ }
  // BFS: prod/optional edges first (prod depth), then dev edges from members.
  const depth = new Map(); const prod = new Set(); const localEdges = new Map();
  const walk = (seeds, devPass) => {
    let q = seeds; let allowDev = devPass;
    while (q.length) {
      const next = [];
      for (const k of q) for (const [t, kind] of g.deps.get(k) || []) {
        if (kind === 1 && !allowDev) continue;
        if (!g.pkgs.has(t)) continue;
        const ek = `${k}>${t}`; const prev = localEdges.get(ek); localEdges.set(ek, prev === undefined ? kind === 1 : prev && kind === 1);
        if (!devPass) prod.add(t);
        if (!depth.has(t)) { depth.set(t, depth.get(k) + 1); next.push(t); }
      }
      q = next; allowDev = false;
    }
  };
  g.members.forEach((m) => { depth.set(m, 0); prod.add(m); });
  walk([...g.members], false);
  walk([...g.members], true);
  const mask = 1 << ri; const byDepth = {}; let devOnly = 0;
  for (const [k, d] of depth) { const n = nodesOut[idxOf(k, g.pkgs.get(k), R.id)]; n.r[R.id] = Math.min(n.r[R.id] ?? 99, d); if (!prod.has(k)) { (n.x ||= {})[R.id] = 1; devOnly++; } byDepth[d] = (byDepth[d] || 0) + 1; }
  for (const [ek, dev] of localEdges) {
    const [a, b] = ek.split('>'); if (!depth.has(a)) continue;
    const ia = idxOf(a, g.pkgs.get(a), R.id), ib = idxOf(b, g.pkgs.get(b), R.id), k = `${ia}>${ib}`;
    const e = edgeMap.get(k) || [ia, ib, 0, dev ? 1 : 0]; e[2] |= mask; e[3] = e[3] && dev ? 1 : 0; edgeMap.set(k, e);
  }
  const memberIdx = g.members.filter((m) => depth.has(m)).map((m) => idxOf(m, g.pkgs.get(m), R.id));
  roots.push({ id: R.id, label: R.label || R.id, bit: ri, url: R.url || '', lockfile: g.lockfile, commit, members: memberIdx,
    counts: { total: depth.size, members: memberIdx.length, direct: byDepth[1] || 0, transitive: depth.size - memberIdx.length - (byDepth[1] || 0), devOnly, byDepth } });
  console.log(`${R.id.padEnd(20)} ${String(depth.size).padStart(5)} packages  (${memberIdx.length} members, ${byDepth[1] || 0} direct, ${devOnly} dev-only, max depth ${Math.max(0, ...Object.keys(byDepth).map(Number))})  ${g.lockfile}`);
});
const shared = nodesOut.filter((c) => Object.keys(c.r).length > 1).length;
const data = { generated_at: new Date().toISOString().replace(/:\d\d\.\d+Z$/, 'Z'),
  generator: 'terp-rs tools/gen-tools/js-deps/gen-js-deps.mjs (package.json + pnpm-lock.yaml / package-lock.json; dependencies + optional, devDependencies of workspace members)',
  edge_format: '[from, to, rootBitmask, devOnly]', roots, counts: { packages: nodesOut.length, edges: edgeMap.size, shared }, packages: nodesOut, edges: [...edgeMap.values()] };
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, JSON.stringify(data));
console.log(`wrote ${path.basename(out)}: ${nodesOut.length} unique packages, ${shared} shared by 2+ roots, ${edgeMap.size} edges, ${(fs.statSync(out).size / 1024).toFixed(0)} KB`);
