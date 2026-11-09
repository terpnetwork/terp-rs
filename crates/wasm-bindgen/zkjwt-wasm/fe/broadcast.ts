/**
 * G3 FE → wallet waist. Turns envelope into signable pieces.
 * Does not broadcast. Does not invent Axiom/KZG.
 */

import {
  assertNotExecuteAuthenticate,
  type Envelope,
  type ZkJwtAuthPayload,
} from "./envelope.ts";

export const MSG_EXECUTE = "/cosmwasm.wasm.v1.MsgExecuteContract";

export type WasmExecuteMsg = {
  typeUrl: typeof MSG_EXECUTE;
  value: {
    sender: string;
    contract: string;
    msg: Uint8Array;
    funds: never[];
  };
};

export type SaSelectedData = {
  authenticatorType: "CosmwasmAuthenticatorV1";
  /** Bytes the ante CosmwasmAuthenticator reads as AuthSudoMsg::Authenticate data. */
  selectedAuthenticatorData: Uint8Array;
  payload: ZkJwtAuthPayload;
};

const enc = new TextEncoder();

export function registerClaimExecuteMsg(
  sender: string,
  env: Envelope,
): WasmExecuteMsg {
  assertNotExecuteAuthenticate(env.execute.msg);
  if (!sender) throw new Error("sender required");
  return {
    typeUrl: MSG_EXECUTE,
    value: {
      sender,
      contract: env.execute.contract,
      msg: enc.encode(JSON.stringify(env.execute.msg)),
      funds: [],
    },
  };
}

export function saAnteSelectedData(env: Envelope): SaSelectedData {
  const payload = env.saTx.selectedAuthenticatorData;
  assertNotExecuteAuthenticate(payload);
  return {
    authenticatorType: env.saTx.addAuthenticator,
    selectedAuthenticatorData: enc.encode(JSON.stringify(payload)),
    payload,
  };
}

/** One session: register_claim wasm execute + SA selected data. No execute authenticate. */
export function signableFromEnvelope(sender: string, env: Envelope) {
  return {
    registerClaim: registerClaimExecuteMsg(sender, env),
    saAuth: saAnteSelectedData(env),
  };
}
