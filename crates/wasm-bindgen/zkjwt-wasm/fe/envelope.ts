/**
 * G3 FE waist — one envelope, two planes.
 *
 * Bind / register_claim: CosmWasm execute.
 * Auth: smart-account tx selected data = ZkJwtAuthPayload (ante AuthSudoMsg::Authenticate).
 * Never: wasm execute { authenticate: ... }.
 */

export const CIRCUIT_ID = "zkjwt.passkey.flock.v1";
export const AUTH_TYPE = "CosmwasmAuthenticatorV1";

export type RegisterClaimExecute = {
  register_claim: {
    claim_commitment: string;
    issuer: string;
  };
};

export type ZkJwtAuthPayload = {
  issuer: string;
  claim_commitment: string;
  public_inputs: string;
  proof: string;
  circuit_id: string;
};

export type Envelope = {
  execute: { contract: string; msg: RegisterClaimExecute };
  saTx: {
    addAuthenticator: typeof AUTH_TYPE;
    selectedAuthenticatorData: ZkJwtAuthPayload;
  };
};

export function registerClaimMsg(
  claimCommitmentB64: string,
  issuer: string,
): RegisterClaimExecute {
  if (!claimCommitmentB64) throw new Error("claim_commitment required");
  if (!issuer) throw new Error("issuer required");
  return { register_claim: { claim_commitment: claimCommitmentB64, issuer } };
}

export function zkJwtAuthPayload(p: Omit<ZkJwtAuthPayload, "circuit_id"> & { circuit_id?: string }): ZkJwtAuthPayload {
  if (!p.proof) throw new Error("proof required");
  if (!p.public_inputs) throw new Error("public_inputs required");
  return {
    issuer: p.issuer,
    claim_commitment: p.claim_commitment,
    public_inputs: p.public_inputs,
    proof: p.proof,
    circuit_id: p.circuit_id ?? CIRCUIT_ID,
  };
}

/** Same envelope: register_claim execute + SA tx (not execute authenticate). */
export function sameEnvelope(args: {
  contract: string;
  claimCommitmentB64: string;
  issuer: string;
  publicInputsB64: string;
  proofB64: string;
}): Envelope {
  const msg = registerClaimMsg(args.claimCommitmentB64, args.issuer);
  const selectedAuthenticatorData = zkJwtAuthPayload({
    issuer: args.issuer,
    claim_commitment: args.claimCommitmentB64,
    public_inputs: args.publicInputsB64,
    proof: args.proofB64,
  });
  return {
    execute: { contract: args.contract, msg },
    saTx: { addAuthenticator: AUTH_TYPE, selectedAuthenticatorData },
  };
}

/** Refuse the dead execute-auth path. */
export function assertNotExecuteAuthenticate(msg: unknown): void {
  if (msg && typeof msg === "object" && "authenticate" in (msg as object)) {
    throw new Error("ExecuteMsg::Authenticate is gone; use SA tx + ante AuthSudoMsg::Authenticate");
  }
}
