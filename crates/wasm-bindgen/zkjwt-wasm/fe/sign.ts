/**
 * One signAndBroadcast waist. Inject a real signer later.
 * Order: register_claim execute, then SA tx with selected ZkJwtAuthPayload.
 * Never wasm execute authenticate.
 */

import type { Envelope } from "./envelope.ts";
import { assertNotExecuteAuthenticate } from "./envelope.ts";
import { signableFromEnvelope, type SaSelectedData, type WasmExecuteMsg } from "./broadcast.ts";

export type SignedTx = { hash: string; kind: "register_claim" | "sa_auth" };

export type Signer = {
  address: string;
  broadcastExecute: (msg: WasmExecuteMsg) => Promise<SignedTx>;
  /** SA tx: selected authenticator data rides ante, not contract execute. */
  broadcastSaTx: (auth: SaSelectedData, memo?: string) => Promise<SignedTx>;
};

export async function signAndBroadcast(signer: Signer, env: Envelope): Promise<SignedTx[]> {
  assertNotExecuteAuthenticate(env.execute.msg);
  const { registerClaim, saAuth } = signableFromEnvelope(signer.address, env);
  const claim = await signer.broadcastExecute(registerClaim);
  if (claim.kind !== "register_claim") {
    throw new Error("first tx must be register_claim execute");
  }
  const auth = await signer.broadcastSaTx(saAuth);
  if (auth.kind !== "sa_auth") {
    throw new Error("second tx must be SA ante auth, not execute authenticate");
  }
  return [claim, auth];
}
