// zafu-client: the documented Penumbra web-extension client path (see docs/V2-SPEC.md §Zafu).
// provider.connect() -> MessagePort -> @penumbra-zone/transport-dom createChannelTransport -> connect-rpc ViewService client.
import { createChannelTransport } from "@penumbra-zone/transport-dom/create";
import { createClient } from "@connectrpc/connect";
import { createRegistry } from "@bufbuild/protobuf";
import { ViewService } from "@penumbra-zone/protobuf/penumbra/view/v1/view_connect";
import { bech32mAddress, addressFromBech32m } from "@penumbra-zone/bech32m/penumbra";

export const PenumbraSymbol = Symbol.for("penumbra");
export const ADDRESS_INNER_LEN = 80; // 16 diversifier + 32 transmission key + 32 clue key

/** Every injected provider, keyed by extension origin (chrome-extension://<id>). */
export function providers(win = globalThis) {
  const g = win[PenumbraSymbol]; if (!g || typeof g !== "object") return [];
  return Object.keys(g).filter((o) => /^chrome-extension:\/\/[a-p]{32}$/.test(o) && g[o] && typeof g[o].connect === "function").map((origin) => ({ origin, provider: g[origin] }));
}

export function makeViewClient(provider, timeoutMs = 120000) {
  const transport = createChannelTransport({ getPort: () => provider.connect(), jsonOptions: { typeRegistry: createRegistry(ViewService) }, defaultTimeoutMs: timeoutMs });
  return createClient(ViewService, transport);
}

export const encodeAddress = (inner) => bech32mAddress({ inner });
export const decodeAddress = (s) => addressFromBech32m(s).inner;

const eqBytes = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

/**
 * Ask the wallet for the address at exactly `index` = {account, randomizer(12 bytes)} and prove it:
 * 1. AddressByIndex(index) -> address (80-byte inner)
 * 2. bech32m round trip (encode -> decode gives the same 80 bytes, hrp "penumbra")
 * 3. IndexByAddress(address) returns the same {account, randomizer} (the wallet owns it at that index)
 */
export async function addressAtIndex(client, { account, randomizer }) {
  if (!Number.isInteger(account) || account < 0 || account > 0xffffffff) throw new Error("account must be a u32");
  if (!(randomizer instanceof Uint8Array) || randomizer.length !== 12) throw new Error("randomizer must be 12 bytes");
  const res = await client.addressByIndex({ addressIndex: { account, randomizer } });
  const inner = res.address?.inner;
  if (!inner || inner.length !== ADDRESS_INNER_LEN) throw new Error("wallet returned a malformed address");
  const text = encodeAddress(inner);
  if (!text.startsWith("penumbra1") || !eqBytes(decodeAddress(text), inner)) throw new Error("address failed the bech32m round trip");
  const back = await client.indexByAddress({ address: { inner } });
  const got = back.addressIndex;
  if (!got) throw new Error("wallet does not recognise the address it returned");
  const gotR = got.randomizer && got.randomizer.length ? got.randomizer : new Uint8Array(12);
  if ((got.account ?? 0) !== account || !eqBytes(gotR, randomizer)) throw new Error("IndexByAddress returned a different index");
  return { address: text, account, randomizer };
}
