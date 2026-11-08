// Headless check of lib/svg-sanitize.js link rules. 127.0.0.1 only; starts and stops its own
// static server + chrome-headless-shell. Usage: node run-sanitize-test.mjs <site_dir> <example.svg>
import { spawn } from 'child_process';
import fs from 'fs'; import os from 'os'; import path from 'path';
const [site, example] = process.argv.slice(2);
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'sanit-'));
fs.symlinkSync(path.join(site, 'lib'), path.join(root, 'lib'));
fs.copyFileSync(new URL('./sanitize-links.test.html', import.meta.url), path.join(root, 'index.html'));
fs.copyFileSync(example, path.join(root, 'example.svg'));
const srv = spawn('/usr/bin/python3', ['-m', 'http.server', '4417', '--bind', '127.0.0.1', '--directory', root], { stdio: 'ignore' });
const bin = process.env.CHROME || (os.homedir() + '/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell');
const ch = spawn(bin, ['--headless', '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=9334', '--user-data-dir=' + path.join(root, 'chrome'), 'about:blank'], { stdio: 'ignore' });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let code = 1;
try {
  let tabs; for (let i = 0; i < 40; i++) { try { tabs = await (await fetch('http://127.0.0.1:9334/json')).json(); break; } catch { await sleep(300); } }
  const ws = new WebSocket(tabs.find((t) => t.type === 'page').webSocketDebuggerUrl);
  await new Promise((r) => (ws.onopen = r));
  let id = 0; const pend = new Map();
  ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pend.has(d.id)) { pend.get(d.id)(d); pend.delete(d.id); } };
  const send = (method, params = {}) => new Promise((r) => { const i = ++id; pend.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
  await send('Page.enable'); await send('Page.navigate', { url: 'http://127.0.0.1:4417/index.html' });
  let res;
  for (let i = 0; i < 40 && !res; i++) { await sleep(250); res = (await send('Runtime.evaluate', { expression: 'window.__results', returnByValue: true })).result?.result?.value; }
  for (const r of res || []) console.log((r.ok ? 'ok   ' : 'FAIL ') + r.name);
  const fails = (res || []).filter((r) => !r.ok).length;
  console.log(res ? `${res.length - fails} passed, ${fails} failed` : 'no results');
  code = res && !fails ? 0 : 1;
} finally { ch.kill(); srv.kill(); await sleep(300); fs.rmSync(root, { recursive: true, force: true }); }
process.exit(code);
