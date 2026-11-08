import { CONTRACTS } from "./catalog.js";

const COSMES = "https://esm.sh/@goblinhunt/cosmes@0.0.71-ghunt.21";
const STORE = "terp-web/v1";

const $ = (id) => document.getElementById(id);

const els = {
  stateSource: $("state-source"),
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
  instances: $("instances"),
  instancesStatus: $("instances-status"),
  loadInstances: $("load-instances"),
  moreInstances: $("more-instances"),
  token: $("token"),
  routes: $("routes"),
  channels: $("channels"),
  pathsStatus: $("paths-status"),
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
let stateIndex = [];
let schemaDoc = null;
const schemaCache = new Map();
let selected = null;
let bundle = null;
let wallet = null;
const bundleCache = new Map();
let instanceAbort = null;
let instanceNext = null;
let ibcDoc = null;
let liveChannels = null;

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

function withCodeId() {
  return CONTRACTS.filter((contract) => deployment(contract.id).codeId != null);
}

function renderList() {
  const q = els.filter.value.trim().toLowerCase();
  const rows = withCodeId().filter((contract) => !q || contract.id.includes(q));
  els.list.replaceChildren();
  if (rows.length === 0) {
    const item = document.createElement("li");
    item.className = "empty";
    item.textContent = q ? "No match." : "This file has no code id for these contracts.";
    els.list.append(item);
    return;
  }
  for (const contract of rows) {
    const dep = deployment(contract.id);
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.id = contract.id;
    button.setAttribute("aria-current", selected?.id === contract.id ? "true" : "false");
    const name = document.createElement("span");
    name.textContent = contract.id;
    const bits = [`code ${dep.codeId}`];
    if (dep.address) bits.push(dep.address.slice(0, 18) + "…");
    const detail = document.createElement("small");
    detail.textContent = bits.join(" · ");
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

function variantInner(schema, snake) {
  if (!schema) return null;
  const branches = schema.oneOf || schema.anyOf || [schema];
  for (const branch of branches) {
    if (branch?.properties && Object.prototype.hasOwnProperty.call(branch.properties, snake)) {
      return branch.properties[snake];
    }
  }
  return null;
}

function stringExample(schema) {
  const blob = `${schema.format || ""} ${schema.title || ""} ${schema.description || ""}`.toLowerCase();
  if (blob.includes("uint") || blob.includes("decimal") || blob.includes("coin")) return "0";
  return "";
}

function exampleValue(schema, root, seen = new Set()) {
  if (!schema || typeof schema !== "object") return {};
  if (schema.$ref) {
    const name = String(schema.$ref).replace(/^#\/definitions\//, "");
    const def = root?.definitions?.[name];
    if (!def || seen.has(name)) return stringExample(schema);
    const next = new Set(seen);
    next.add(name);
    return exampleValue(def, root, next);
  }
  if (Object.prototype.hasOwnProperty.call(schema, "const")) return schema.const;
  if (Array.isArray(schema.enum) && schema.enum.length) return schema.enum[0];
  const alts = schema.oneOf || schema.anyOf;
  if (alts?.length) return exampleValue(alts[0], root, seen);
  const type = Array.isArray(schema.type) ? schema.type.find((item) => item !== "null") : schema.type;
  if (type === "string") return stringExample(schema);
  if (type === "integer" || type === "number") return 0;
  if (type === "boolean") return false;
  if (type === "array") return [];
  if (type === "object" || schema.properties) {
    const out = {};
    const props = schema.properties || {};
    for (const key of schema.required || []) {
      out[key] = props[key] ? exampleValue(props[key], root, seen) : null;
    }
    return out;
  }
  return {};
}

function payloadFor(schema, snake) {
  if (!schema || !snake) return {};
  const inner = variantInner(schema, snake);
  if (!inner) return {};
  const value = exampleValue(inner, schema);
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function writePayload(kind) {
  if (!bundle) return;
  const select = kind === "query" ? els.queryVariant : els.execVariant;
  const textarea = kind === "query" ? els.queryBody : els.execBody;
  const table = kind === "query" ? bundle.Query : bundle.Execute;
  const schema = kind === "query" ? schemaDoc?.query : schemaDoc?.execute;
  textarea.value = JSON.stringify(payloadFor(schema, table?.[select.value]), null, 2);
}

async function loadSchema(id) {
  if (schemaCache.has(id)) return schemaCache.get(id);
  try {
    const doc = await readJson("/schemas/" + encodeURIComponent(id));
    schemaCache.set(id, doc);
    return doc;
  } catch {
    schemaCache.set(id, null);
    return null;
  }
}

function clearInstances() {
  instanceAbort?.abort();
  instanceAbort = null;
  instanceNext = null;
  els.instances.replaceChildren();
  els.instancesStatus.textContent = "";
  els.loadInstances.disabled = true;
  els.moreInstances.disabled = true;
}

function appendInstance(address) {
  const item = document.createElement("li");
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = address;
  button.setAttribute("aria-current", els.address.value.trim() === address ? "true" : "false");
  button.addEventListener("click", () => {
    els.address.value = address;
    els.address.dispatchEvent(new Event("change"));
    for (const other of els.instances.querySelectorAll("button")) {
      other.setAttribute("aria-current", other === button ? "true" : "false");
    }
  });
  item.append(button);
  els.instances.append(item);
}

async function loadInstances(reset) {
  if (!selected) return;
  const codeId = deployment(selected.id).codeId;
  if (codeId == null) return;
  if (reset) {
    instanceAbort?.abort();
    instanceNext = null;
    els.instances.replaceChildren();
  }
  const ctrl = new AbortController();
  instanceAbort = ctrl;
  const params = new URLSearchParams({ "pagination.limit": "50" });
  if (!reset && instanceNext) params.set("pagination.key", instanceNext);
  els.instancesStatus.textContent = reset ? `Reading code ${codeId}…` : "Reading the next page…";
  els.loadInstances.disabled = true;
  try {
    const body = await readJson(
      `${lcdBase()}/cosmwasm/wasm/v1/code/${codeId}/contracts?${params}`,
      { signal: ctrl.signal },
    );
    if (ctrl.signal.aborted) return;
    const rows = Array.isArray(body.contracts) ? body.contracts : [];
    instanceNext = body.pagination?.next_key || null;
    for (const address of rows) appendInstance(address);
    const shown = els.instances.querySelectorAll("button").length;
    els.instancesStatus.textContent = shown
      ? `Code ${codeId}. ${shown} contract${shown === 1 ? "" : "s"}${instanceNext ? ". More pages remain." : "."}`
      : `Code ${codeId} has no contracts on this REST node.`;
    els.moreInstances.disabled = !instanceNext;
  } catch (err) {
    if (err?.name === "AbortError") return;
    els.instancesStatus.textContent = err instanceof Error ? err.message : String(err);
    els.moreInstances.disabled = true;
  } finally {
    if (instanceAbort === ctrl) els.loadInstances.disabled = !selected;
  }
}

async function selectContract(id) {
  selected = CONTRACTS.find((contract) => contract.id === id) || null;
  bundle = null;
  schemaDoc = null;
  clearInstances();
  renderList();
  if (!selected) return;
  if (deployment(selected.id).codeId == null) {
    selected = null;
    return;
  }
  els.title.textContent = selected.id;
  const dep = deployment(selected.id);
  els.address.disabled = false;
  els.address.value = dep.address || "";
  els.meta.textContent = `code id ${dep.codeId}`;
  for (const el of [els.queryVariant, els.queryBody, els.execVariant, els.execBody, els.funds, els.readInfo, els.runQuery, els.runExec]) {
    el.disabled = true;
  }
  try {
    bundle = await loadBundle(selected);
    schemaDoc = await loadSchema(selected.id);
    fillVariants(els.queryVariant, bundle.Query);
    fillVariants(els.execVariant, bundle.Execute);
    writePayload("query");
    writePayload("execute");
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
    els.loadInstances.disabled = false;
    show(`Loaded ${bundle.CONTRACT?.name || selected.id}.`);
    loadInstances(true);
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

function hopsOf(trace) {
  return String(trace || "")
    .split("/")
    .filter((part) => part.startsWith("channel-"));
}

function terpSide(entry) {
  if (entry.chain_2?.chain_name === "terp" || entry.chain_2?.chain_id === "morocco-1") return "chain_2";
  if (entry.chain_1?.chain_name === "terp" || entry.chain_1?.chain_id === "morocco-1") return "chain_1";
  return null;
}

function otherSide(side) {
  return side === "chain_1" ? "chain_2" : "chain_1";
}

function liveById() {
  const map = new Map();
  for (const channel of liveChannels || []) {
    if (channel.channel_id) map.set(channel.channel_id, channel);
  }
  return map;
}

function hopClass(channelId) {
  if (!liveChannels) return "";
  const found = liveById().get(channelId);
  if (!found) return "shut";
  return found.state === "STATE_OPEN" ? "open" : "shut";
}

function renderRoutes() {
  const assets = ibcDoc?.tokens?.assets || {};
  const asset = assets[els.token.value];
  els.routes.replaceChildren();
  if (!asset) return;
  const live = liveById();
  for (const variant of asset.variants || []) {
    const hops = hopsOf(variant.trace_path);
    const block = document.createElement("div");
    block.className = "route";
    const title = document.createElement("h3");
    title.textContent = variant.via || variant.trace_path;
    const meta = document.createElement("p");
    meta.className = "status";
    const first = hops[0];
    const seen = liveChannels ? live.get(first) : null;
    const liveNote = !liveChannels
      ? ""
      : seen
        ? `Terp ${first} is ${seen.state}.`
        : `Terp ${first} is not on this REST node.`;
    meta.textContent = [variant.status, liveNote].filter(Boolean).join(" · ");
    const hopRow = document.createElement("div");
    hops.forEach((hop, index) => {
      const pill = document.createElement("span");
      pill.className = `hop mono${index === 0 ? ` ${hopClass(hop)}` : ""}`.trim();
      pill.textContent = index === 0 ? hop : `then ${hop}`;
      hopRow.append(pill);
    });
    const denom = document.createElement("p");
    denom.className = "mono";
    denom.style.fontSize = "12px";
    denom.textContent = variant.ibc_denom || "";
    block.append(title, meta, hopRow);
    if (variant.note) {
      const note = document.createElement("p");
      note.className = "status";
      note.textContent = variant.note;
      block.append(note);
    }
    block.append(denom);
    els.routes.append(block);
  }
}

function renderChannelList() {
  els.channels.replaceChildren();
  if (liveChannels) {
    const rows = [...liveChannels].sort((a, b) => {
      const na = Number(String(a.channel_id).replace("channel-", ""));
      const nb = Number(String(b.channel_id).replace("channel-", ""));
      return na - nb;
    });
    for (const channel of rows) {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      const name = document.createElement("span");
      name.textContent = `${channel.channel_id} · ${channel.port_id}`;
      const detail = document.createElement("small");
      const other = channel.counterparty?.channel_id || "";
      detail.textContent = [channel.state, other && `counterparty ${other}`].filter(Boolean).join(" · ");
      button.append(name, detail);
      item.append(button);
      els.channels.append(item);
    }
    return;
  }
  for (const entry of ibcDoc?.paths || []) {
    const side = terpSide(entry);
    const counterparty = side ? entry[otherSide(side)] : entry.chain_1;
    for (const channel of entry.channels || []) {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      const ours = side ? channel[side]?.channel_id : "";
      const theirs = side ? channel[otherSide(side)]?.channel_id : "";
      const name = document.createElement("span");
      name.textContent = `${counterparty?.chain_name || entry.id} · ${ours || "(no terp channel)"}`;
      const detail = document.createElement("small");
      detail.textContent = [
        entry.client_status,
        channel.tags?.status,
        theirs && `their ${theirs}`,
      ].filter(Boolean).join(" · ");
      button.append(name, detail);
      item.append(button);
      els.channels.append(item);
    }
  }
}

async function ensurePaths() {
  if (ibcDoc) return;
  els.pathsStatus.textContent = "Loading routes…";
  try {
    ibcDoc = await readJson("/ibc");
    const assets = ibcDoc.tokens?.assets || {};
    els.token.replaceChildren();
    for (const [id, asset] of Object.entries(assets)) {
      const option = document.createElement("option");
      option.value = id;
      option.textContent = asset.canonical || id;
      els.token.append(option);
    }
    const openCurated = (ibcDoc.paths || []).filter((entry) => entry.client_status === "Active").length;
    els.pathsStatus.textContent = `${Object.keys(assets).length} tokens. ${openCurated} counterparties marked Active in the curated files.`;
    renderRoutes();
    renderChannelList();
  } catch (err) {
    els.pathsStatus.textContent = err instanceof Error ? err.message : String(err);
  }
}

async function readChannels() {
  const button = $("read-channels");
  button.disabled = true;
  els.pathsStatus.textContent = "Reading transfer channels…";
  try {
    const all = [];
    let key = null;
    for (let page = 0; page < 20; page += 1) {
      const params = new URLSearchParams({ "pagination.limit": "100" });
      if (key) params.set("pagination.key", key);
      const body = await readJson(`${lcdBase()}/ibc/core/channel/v1/channels?${params}`);
      all.push(...(Array.isArray(body.channels) ? body.channels : []));
      key = body.pagination?.next_key || null;
      if (!key) break;
    }
    liveChannels = all.filter((channel) => channel.port_id === "transfer");
    const open = liveChannels.filter((channel) => channel.state === "STATE_OPEN").length;
    els.pathsStatus.textContent = `${liveChannels.length} transfer channels on this REST node. ${open} are STATE_OPEN. Token hops use the first channel.`;
    renderRoutes();
    renderChannelList();
  } catch (err) {
    els.pathsStatus.textContent = err instanceof Error ? err.message : String(err);
  } finally {
    button.disabled = false;
  }
}

function showTab(name) {
  for (const button of document.querySelectorAll("#tabs button")) {
    const on = button.dataset.tab === name;
    button.setAttribute("aria-selected", on ? "true" : "false");
    button.classList.toggle("ghost", !on);
  }
  $("panel-contracts").hidden = name !== "contracts";
  $("panel-paths").hidden = name !== "paths";
  if (name === "paths") ensurePaths();
}

function applyPreset(name) {
  const preset = PRESETS[name];
  els.rpc.value = preset.rpc;
  els.lcd.value = preset.lcd;
  els.gas.value = preset.gas;
  remember({ rpc: preset.rpc, lcd: preset.lcd, gas: preset.gas });
}

function bind() {
  els.queryVariant.addEventListener("change", () => writePayload("query"));
  els.execVariant.addEventListener("change", () => writePayload("execute"));
  els.stateSource.addEventListener("change", async () => {
    remember({ state: els.stateSource.value });
    try {
      await applyState(els.stateSource.value);
    } catch (err) {
      setStatus(err instanceof Error ? err.message : String(err), true);
    }
  });
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
  els.loadInstances.addEventListener("click", () => loadInstances(true));
  els.moreInstances.addEventListener("click", () => loadInstances(false));
  els.token.addEventListener("change", renderRoutes);
  $("read-channels").addEventListener("click", readChannels);
  for (const button of document.querySelectorAll("#tabs button")) {
    button.addEventListener("click", () => showTab(button.dataset.tab));
  }
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

function stateOptionLabel(row) {
  const hot = (row.chains || []).filter((chain) => chain.codes || chain.addresses);
  const bits = hot.slice(0, 3).map((chain) => `${chain.id} ${chain.addresses} addr`);
  const more = hot.length > 3 ? ` +${hot.length - 3}` : "";
  const missing = row.present ? "" : " missing";
  return `${row.label}${missing}${bits.length ? " · " + bits.join(", ") + more : ""}`;
}

function fillStateSources() {
  const prefer = saved().state;
  els.stateSource.replaceChildren();
  for (const row of stateIndex) {
    const option = document.createElement("option");
    option.value = row.id;
    option.textContent = stateOptionLabel(row);
    option.disabled = !row.present;
    option.selected = row.id === prefer;
    els.stateSource.append(option);
  }
}

async function applyState(id) {
  stateDoc = await readJson("/states/" + encodeURIComponent(id));
  fillChains();
  const row = stateIndex.find((item) => item.id === id);
  setStatus(`${row?.label || id}. ${chainIds().length} chains. ${withCodeId().length} contracts have a code id on ${els.chain.value || "(none)"}.`);
  if (selected && deployment(selected.id).codeId == null) {
    selected = null;
    bundle = null;
    schemaDoc = null;
    els.title.textContent = "Pick a contract";
    els.meta.textContent = "";
    els.address.value = "";
    clearInstances();
    renderList();
    return;
  }
  if (selected) await selectContract(selected.id);
}

async function main() {
  bind();
  const memory = saved();
  if (memory.rpc) els.rpc.value = memory.rpc;
  if (memory.lcd) els.lcd.value = memory.lcd;
  if (memory.gas) els.gas.value = memory.gas;
  try {
    stateIndex = await readJson("/states");
    fillStateSources();
    const preferred =
      stateIndex.find((row) => row.id === saved().state && row.present) ||
      stateIndex.find((row) => row.id === "cw-orch" && row.present) ||
      stateIndex.find((row) => row.present);
    if (!preferred) throw new Error("None of the mapped state.json files are on this machine.");
    els.stateSource.value = preferred.id;
    await applyState(preferred.id);
  } catch (err) {
    setStatus("Could not read a state file. Run just web serve from the terp-rs root. A server started inside web/ cannot see the bundles or ~/.cw-orchestrator.", true);
    show(err instanceof Error ? err.message : String(err));
    renderList();
  }
}

main();
