import { CONTRACTS } from "./catalog.js";

const COSMES = "https://esm.sh/@goblinhunt/cosmes@0.0.71-ghunt.21";
const STATE_URL = "../public/state.json";
const STORE = "terp-web/v1";

const $ = (id) => document.getElementById(id);

const els = {
  chain: $("chain"),
  rpc: $("rpc"),
  lcd: $("lcd"),
  gas: $("gas"),
  status: $("status"),
  list: $("list"),
  filter: $("filter"),
  title: $("title"),
  meta: $("meta"),
  address: $("address"),
  queryVariant: $("query-variant"),
  queryBody: $("query-body"),
  execVariant: $("exec-variant"),
  execBody: $("exec-body"),
  funds: $("funds"),
  out: $("out"),
  connect: $("connect"),
  disconnect: $("disconnect"),
  readInfo: $("read-info"),
  runQuery: $("run-query"),
  runExec: $("run-exec"),
};

const PRESETS = {
  local: {
    rpc: "http://127.0.0.1:26657",
    lcd: "http://127.0.0.1:1317",
    gas: "0.025uterp",
  },
  morocco: {
    rpc: "https://rpc.terp.network",
    lcd: "https://api.terp.network",
    gas: "0.025uterp",
  },
};

let stateDoc = null;
let selected = null;
let bundle = null;
let wallet = null;
const bundleCache = new Map();

function saved() {
  try {
    return JSON.parse(localStorage.getItem(STORE) || "{}");
  } catch {
    return {};
  }
}

function remember(patch) {
  const next = { ...saved(), ...patch };
  localStorage.setItem(STORE, JSON.stringify(next));
}

function norm(value) {
  return String(value)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

function findKey(map, id) {
  if (!map || typeof map !== "object") return null;
  const want = norm(id);
  const keys = Object.keys(map);
  return (
    keys.find((key) => norm(key) === want || norm(key) === `crates-io-${want}`) ||
    keys.find((key) => norm(key).endsWith(`-${want}`)) ||
    null
  );
}

function chainRecord() {
  if (!stateDoc) return null;
  return stateDoc[els.chain.value] || null;
}

function deployment(contractId) {
  const record = chainRecord();
  if (!record) return { codeId: null, address: "" };
  const codeKey = findKey(record.code_ids, contractId);
  const addrKey = findKey(record.default, contractId);
  const override = saved().addresses?.[`${els.chain.value}:${contractId}`];
  return {
    codeId: codeKey ? record.code_ids[codeKey] : null,
    address: override || (addrKey ? record.default[addrKey] : ""),
  };
}

function chainIds() {
  if (!stateDoc) return [];
  return Object.entries(stateDoc)
    .filter(([, value]) => value && typeof value === "object" && (value.code_ids || value.default))
    .map(([id]) => id);
}

function setStatus(text, warn = false) {
  els.status.textContent = text;
  els.status.classList.toggle("warn", warn);
}

function show(value) {
  els.out.textContent = typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function parseJson(text, fallback) {
  const trimmed = text.trim();
  if (!trimmed) return fallback;
  return JSON.parse(trimmed);
}

function parseGas(text) {
  const match = String(text).trim().match(/^([0-9]*\.?[0-9]+)([a-zA-Z][a-zA-Z0-9/._-]*)$/);
  if (!match) throw new Error("Gas price looks like 0.025uterp");
  return { amount: match[1], denom: match[2] };
}

function fillVariants(select, table) {
  select.replaceChildren();
  const names = Object.keys(table || {});
  if (names.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "(none)";
    select.append(option);
    return;
  }
  for (const name of names) {
    const option = document.createElement("option");
    option.value = name;
    option.textContent = `${name} → ${table[name]}`;
    select.append(option);
  }
}

function renderList() {
  const q = els.filter.value.trim().toLowerCase();
  els.list.replaceChildren();
  for (const contract of CONTRACTS) {
    if (q && !contract.id.includes(q)) continue;
    const dep = deployment(contract.id);
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.id = contract.id;
    button.setAttribute("aria-current", selected?.id === contract.id ? "true" : "false");
    const name = document.createElement("span");
    name.textContent = contract.id;
    const detail = document.createElement("small");
    const bits = [];
    if (dep.codeId != null) bits.push(`code ${dep.codeId}`);
    if (dep.address) bits.push(dep.address.slice(0, 18) + "…");
    detail.textContent = bits.join(" · ") || "no state.json entry";
    button.append(name, detail);
    button.addEventListener("click", () => selectContract(contract.id));
    item.append(button);
    els.list.append(item);
  }
}

async function loadBundle(contract) {
  if (bundleCache.has(contract.id)) return bundleCache.get(contract.id);
  const mod = await import(contract.bundle);
  bundleCache.set(contract.id, mod);
  return mod;
}

async function selectContract(id) {
  selected = CONTRACTS.find((contract) => contract.id === id) || null;
  bundle = null;
  renderList();
  if (!selected) return;
  els.title.textContent = selected.id;
  const dep = deployment(selected.id);
  els.address.disabled = false;
  els.address.value = dep.address || "";
  els.meta.textContent = dep.codeId != null ? `state.json code id ${dep.codeId}` : "No code id in this chain record.";
  for (const el of [els.queryVariant, els.queryBody, els.execVariant, els.execBody, els.funds, els.readInfo, els.runQuery, els.runExec]) {
    el.disabled = true;
  }
  try {
    bundle = await loadBundle(selected);
    fillVariants(els.queryVariant, bundle.Query);
    fillVariants(els.execVariant, bundle.Execute);
    const canQuery = Object.keys(bundle.Query || {}).length > 0;
    const canExec = Object.keys(bundle.Execute || {}).length > 0;
    els.queryVariant.disabled = !canQuery;
    els.queryBody.disabled = !canQuery;
    els.runQuery.disabled = !canQuery;
    els.execVariant.disabled = !canExec;
    els.execBody.disabled = !canExec;
    els.funds.disabled = !canExec;
    els.runExec.disabled = !canExec;
    els.readInfo.disabled = false;
    show(`Loaded ${bundle.CONTRACT?.name || selected.id}.`);
  } catch (err) {
    show(err instanceof Error ? err.stack || err.message : String(err));
  }
}

function lcdBase() {
  return els.lcd.value.trim().replace(/\/$/, "");
}

async function readJson(url, init) {
  const response = await fetch(url, init);
  const text = await response.text();
  let body;
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    body = { raw: text };
  }
  if (!response.ok) {
    const message = body.message || body.raw || response.statusText;
    throw new Error(`${response.status} ${message}`);
  }
  return body;
}

function bytesToBase64(text) {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

async function querySmart(address, msg) {
  const encoded = bytesToBase64(JSON.stringify(msg));
  const url = `${lcdBase()}/cosmwasm/wasm/v1/contract/${address}/smart/${encodeURIComponent(encoded)}`;
  const body = await readJson(url);
  return body.data ?? body;
}

function requireAddress() {
  const address = els.address.value.trim();
  if (!address) throw new Error("Set a contract address. state.json fills one when this chain has a default.");
  return address;
}

async function connectKeplr() {
  if (!window.keplr) {
    setStatus("Keplr is not in this browser. Queries still work.", true);
    return;
  }
  els.connect.disabled = true;
  setStatus("Loading wallet support…");
  try {
    const chainId = els.chain.value;
    const gas = parseGas(els.gas.value);
    const suggest = {
      chainId,
      chainName: chainId,
      rpc: els.rpc.value.trim(),
      rest: lcdBase(),
      bip44: { coinType: 118 },
      bech32Config: prefixConfig("terp"),
      currencies: [coin("TERP", gas.denom)],
      feeCurrencies: [{ ...coin("TERP", gas.denom), gasPriceStep: { low: 0.01, average: Number(gas.amount) || 0.025, high: 0.04 } }],
      stakeCurrency: coin("TERP", gas.denom),
    };
    if (window.keplr.experimentalSuggestChain) {
      await window.keplr.experimentalSuggestChain(suggest);
    }
    const [{ KeplrController }, { MsgExecuteContract }] = await Promise.all([
      import(`${COSMES}/wallet`),
      import(`${COSMES}/client`),
    ]);
    const controller = new KeplrController("");
    const connected = await controller.connect("extension", [
      { chainId, rpc: suggest.rpc, gasPrice: { amount: gas.amount, denom: gas.denom } },
    ]);
    const account = connected.get(chainId);
    if (!account) throw new Error(`Keplr did not return ${chainId}`);
    wallet = { account, MsgExecuteContract, controller };
    els.disconnect.hidden = false;
    setStatus(`Keplr ${account.address}`);
  } catch (err) {
    wallet = null;
    setStatus(err instanceof Error ? err.message : String(err), true);
  } finally {
    els.connect.disabled = false;
  }
}

function prefixConfig(prefix) {
  return {
    bech32PrefixAccAddr: prefix,
    bech32PrefixAccPub: `${prefix}pub`,
    bech32PrefixValAddr: `${prefix}valoper`,
    bech32PrefixValPub: `${prefix}valoperpub`,
    bech32PrefixConsAddr: `${prefix}valcons`,
    bech32PrefixConsPub: `${prefix}valconspub`,
  };
}

function coin(display, denom) {
  return { coinDenom: display, coinMinimalDenom: denom, coinDecimals: 6 };
}

function fundsOf(text) {
  const rows = parseJson(text, []);
  if (!Array.isArray(rows)) throw new Error("Funds must be a JSON array");
  return rows.map((row) => ({ denom: row.denom, amount: BigInt(row.amount) }));
}

async function runQuery() {
  if (!bundle || !selected) return;
  const method = els.queryVariant.value;
  els.runQuery.disabled = true;
  try {
    const address = requireAddress();
    const payload = parseJson(els.queryBody.value, {});
    const api = bundle.createApi(
      { queryContractSmart: (_addr, msg) => querySmart(address, msg) },
      address,
    );
    const result = await api.query[method](payload);
    show(result);
  } catch (err) {
    show(err instanceof Error ? err.message : String(err));
  } finally {
    els.runQuery.disabled = false;
  }
}

async function runExec() {
  if (!bundle || !selected) return;
  if (!wallet) {
    setStatus("Connect Keplr before broadcasting.", true);
    return;
  }
  const method = els.execVariant.value;
  els.runExec.disabled = true;
  try {
    const address = requireAddress();
    const payload = parseJson(els.execBody.value, {});
    const funds = fundsOf(els.funds.value);
    const sender = wallet.account.address;
    const api = bundle.createApi(
      {
        queryContractSmart: (_addr, msg) => querySmart(address, msg),
        execute: async (_sender, contract, msg) => {
          const tx = await wallet.account.broadcastTxSync({
            msgs: [
              new wallet.MsgExecuteContract({
                sender,
                contract,
                msg,
                funds,
              }),
            ],
          });
          return tx;
        },
      },
      address,
    );
    const result = await api.execute[method](sender, payload, funds);
    show(result);
  } catch (err) {
    show(err instanceof Error ? err.message : String(err));
  } finally {
    els.runExec.disabled = false;
  }
}

async function readInfo() {
  els.readInfo.disabled = true;
  try {
    const address = requireAddress();
    const body = await readJson(`${lcdBase()}/cosmwasm/wasm/v1/contract/${address}`);
    show(body.contract_info || body);
  } catch (err) {
    show(err instanceof Error ? err.message : String(err));
  } finally {
    els.readInfo.disabled = false;
  }
}

function applyPreset(name) {
  const preset = PRESETS[name];
  els.rpc.value = preset.rpc;
  els.lcd.value = preset.lcd;
  els.gas.value = preset.gas;
  remember({ rpc: preset.rpc, lcd: preset.lcd, gas: preset.gas });
}

function bind() {
  els.chain.addEventListener("change", () => {
    remember({ chain: els.chain.value });
    renderList();
    if (selected) selectContract(selected.id);
  });
  for (const el of [els.rpc, els.lcd, els.gas]) {
    el.addEventListener("change", () => {
      remember({ rpc: els.rpc.value, lcd: els.lcd.value, gas: els.gas.value });
    });
  }
  els.filter.addEventListener("input", renderList);
  els.address.addEventListener("change", () => {
    if (!selected) return;
    const addresses = { ...(saved().addresses || {}) };
    addresses[`${els.chain.value}:${selected.id}`] = els.address.value.trim();
    remember({ addresses });
    renderList();
  });
  $("preset-local").addEventListener("click", () => applyPreset("local"));
  $("preset-morocco").addEventListener("click", () => applyPreset("morocco"));
  els.connect.addEventListener("click", connectKeplr);
  els.disconnect.addEventListener("click", () => {
    wallet?.controller?.disconnect?.([els.chain.value]);
    wallet = null;
    els.disconnect.hidden = true;
    setStatus("Disconnected. Queries still use REST.");
  });
  els.runQuery.addEventListener("click", runQuery);
  els.runExec.addEventListener("click", runExec);
  els.readInfo.addEventListener("click", readInfo);
  $("state-file").addEventListener("change", async (event) => {
    const file = event.target.files?.[0];
    if (!file) return;
    stateDoc = JSON.parse(await file.text());
    fillChains();
    setStatus(`Loaded ${file.name}. Kept in this tab only.`);
  });
}

function fillChains() {
  const ids = chainIds();
  const prefer = saved().chain && ids.includes(saved().chain) ? saved().chain : ids.includes("morocco-1") ? "morocco-1" : ids[0];
  els.chain.replaceChildren();
  for (const id of ids) {
    const option = document.createElement("option");
    option.value = id;
    option.textContent = id;
    option.selected = id === prefer;
    els.chain.append(option);
  }
  renderList();
}

async function main() {
  bind();
  const memory = saved();
  if (memory.rpc) els.rpc.value = memory.rpc;
  if (memory.lcd) els.lcd.value = memory.lcd;
  if (memory.gas) els.gas.value = memory.gas;
  try {
    stateDoc = await readJson(STATE_URL);
    fillChains();
    const withAddr = CONTRACTS.filter((contract) => deployment(contract.id).address).length;
    setStatus(`${chainIds().length} chains in state.json. ${withAddr} contracts have a default address on ${els.chain.value}.`);
  } catch (err) {
    setStatus(`Could not read ${STATE_URL}. Serve terp-rs as the site root, or pick a state.json file.`, true);
    show(err instanceof Error ? err.message : String(err));
    renderList();
  }
}

main();
