#!/usr/bin/env node
// Rust dependency scraper: the full dependency tree (normal + build, direct and
// transitive, all targets) of one or more cargo roots, from `cargo metadata`,
// merged into one JSON graph (the format public/graphs/rust-crates.html reads).
//
//   node gen-deps.mjs [--out FILE] [--config roots.json] [--root id=PATH]...
//                     [--only id,id] [--offline] [--cache-dir DIR] [--no-clone]
//
// Defaults: --config is roots.json next to this file; --out is
// target/gen-deps/rust-crates.json under the current directory. See README.md.
// Read-only on checkouts, except that cargo may write a Cargo.lock into a
// checkout that has none (only the cache clones).
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const opt = { out: null, config: path.join(here, 'roots.json'), roots: [], only: null, offline: false, clone: true,
  cache: path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache'), 'terp-gen-deps') };
for (let i = 0; i < argv.length; i++) {
  const a = argv[i], v = () => argv[++i];
  if (a === '--out' || a === '-o') opt.out = v();
  else if (a === '--config') opt.config = v();
  else if (a === '--root') opt.roots.push(v());
  else if (a === '--only') opt.only = v().split(',');
  else if (a === '--offline') opt.offline = true;
  else if (a === '--cache-dir') opt.cache = v();
  else if (a === '--no-clone') opt.clone = false;
  else if (a === '-h' || a === '--help') { console.log(fs.readFileSync(fileURLToPath(import.meta.url), 'utf8').split('\nimport')[0]); process.exit(0); }
  else if (!a.startsWith('-') && !opt.out) opt.out = a;
  else { console.error(`unknown argument ${a}`); process.exit(2); }
}
const out = path.resolve(opt.out || 'target/gen-deps/rust-crates.json');
const expand = (s) => String(s).replace(/\$\{(\w+)(?::-([^}]*))?\}/g, (_, k, d) => process.env[k] || d || '').replace(/^~(?=\/|$)/, os.homedir());
const cfgDir = path.dirname(path.resolve(opt.config));
let ROOTS = JSON.parse(fs.readFileSync(opt.config, 'utf8')).roots.map((r) => ({ ...r, path: expand(r.path) ? path.resolve(cfgDir, expand(r.path)) : '' }));
for (const spec of opt.roots) {
  const [id, p] = spec.split('='); const r = ROOTS.find((x) => x.id === id);
  if (r) { r.path = path.resolve(expand(p)); r.sub = ''; } else ROOTS.push({ id, label: id, path: path.resolve(expand(p)) });
}
if (opt.only) ROOTS = ROOTS.filter((r) => opt.only.includes(r.id));
const cargo = process.env.CARGO || [path.join(os.homedir(), '.cargo/bin/cargo'), '/opt/homebrew/bin/cargo'].find((c) => fs.existsSync(c)) || 'cargo';
const sh = (cmd, a, cwd) => execFileSync(cmd, a, { cwd, encoding: 'utf8', maxBuffer: 1 << 30, stdio: ['ignore', 'pipe', 'pipe'] });
const srcKind = (pkg) => (!pkg.source ? 'path' : pkg.source.startsWith('git+') ? 'git' : 'registry');
const tilde = (p) => (p.startsWith(os.homedir()) ? '~' + p.slice(os.homedir().length) : p);

function locate(R) {
  let base = R.path;
  if ((!base || !fs.existsSync(base)) && R.git && opt.clone) {
    base = path.join(opt.cache, R.id);
    if (!fs.existsSync(base)) { console.log(`clone ${R.git} → ${tilde(base)}`); fs.mkdirSync(opt.cache, { recursive: true }); sh('git', ['clone', '--depth', '1', ...(R.rev ? ['--branch', R.rev] : []), R.git, base]); }
  }
  return base ? path.join(base, R.sub || '') : '';
}

const crates = []; const crateIdx = new Map(); const edgeMap = new Map(); const roots = [];
ROOTS.forEach((R, ri) => {
  const dir = locate(R);
  if (!dir || !fs.existsSync(path.join(dir, 'Cargo.toml'))) { console.warn(`skip ${R.id}: no Cargo.toml at ${dir || '(unset)'}`); return; }
  const meta = JSON.parse(sh(cargo, ['metadata', '--format-version', '1', ...(opt.offline ? ['--offline'] : [])], dir));
  let commit = ''; try { commit = sh('git', ['rev-parse', 'HEAD'], dir).trim(); } catch { /* not git */ }
  const pkgs = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const members = new Set(meta.workspace_members);
  const key = (id) => { const p = pkgs.get(id); return `${p.name}@${p.version}#${srcKind(p)}`; };
  const idxOf = (id) => {
    const k = key(id);
    if (!crateIdx.has(k)) { const p = pkgs.get(id); crateIdx.set(k, crates.length); crates.push({ n: p.name, v: p.version, s: srcKind(p), d: (p.description || '').trim().slice(0, 140), r: {} }); }
    return crateIdx.get(k);
  };
  const depth = new Map(); let q = [...members]; q.forEach((m) => depth.set(m, 0)); const localEdges = [];
  while (q.length) {
    const next = [];
    for (const id of q) {
      const n = nodes.get(id); if (!n) continue;
      for (const d of n.deps || []) {
        const kinds = (d.dep_kinds || [{ kind: null }]).map((k) => k.kind);
        if (!kinds.some((k) => k === null || k === 'build')) continue;
        localEdges.push([id, d.pkg, kinds.includes(null) ? 0 : 1]);
        if (!depth.has(d.pkg)) { depth.set(d.pkg, depth.get(id) + 1); next.push(d.pkg); }
      }
    }
    q = next;
  }
  const byDepth = {};
  for (const [id, dp] of depth) { const c = crates[idxOf(id)]; c.r[R.id] = Math.min(c.r[R.id] ?? 99, dp); byDepth[dp] = (byDepth[dp] || 0) + 1; }
  const mask = 1 << ri;
  for (const [a, b, build] of localEdges) {
    const ia = idxOf(a), ib = idxOf(b), k = `${ia}>${ib}`; const e = edgeMap.get(k) || [ia, ib, 0, build];
    e[2] |= mask; e[3] = e[3] && build; edgeMap.set(k, e);
  }
  roots.push({ id: R.id, label: R.label || R.id, bit: ri, upstream: R.upstream || '', commit, members: [...members].map(idxOf),
    counts: { total: depth.size, members: members.size, direct: byDepth[1] || 0, transitive: depth.size - members.size - (byDepth[1] || 0), byDepth } });
  console.log(`${R.id.padEnd(12)} ${String(depth.size).padStart(5)} crates  (${members.size} members, ${byDepth[1] || 0} direct, max depth ${Math.max(...Object.keys(byDepth).map(Number))})  ${commit.slice(0, 10)}`);
});
const shared = crates.filter((c) => Object.keys(c.r).length > 1).length;
const data = { generated_at: new Date().toISOString().replace(/:\d\d\.\d+Z$/, 'Z'),
  generator: 'terp-rs tools/gen-tools/rust-deps/gen-deps.mjs (cargo metadata --format-version 1; normal + build deps, all targets)',
  edge_format: '[from, to, rootBitmask, buildOnly]', roots, counts: { crates: crates.length, edges: edgeMap.size, shared }, crates, edges: [...edgeMap.values()] };
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, JSON.stringify(data));
console.log(`wrote ${tilde(out)}: ${crates.length} unique crates, ${shared} shared by 2+ roots, ${edgeMap.size} edges, ${(fs.statSync(out).size / 1024).toFixed(0)} KB`);
