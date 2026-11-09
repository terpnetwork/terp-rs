import { assertNotExecuteAuthenticate, sameEnvelope, CIRCUIT_ID, AUTH_TYPE } from "./envelope.ts";
import { signableFromEnvelope, MSG_EXECUTE } from "./broadcast.ts";

const env = sameEnvelope({
  contract: "terp1zkjwt",
  claimCommitmentB64: "Y29tbWl0",
  issuer: "terp.network/passkey-zkjwt",
  publicInputsB64: "aW5wdXRz",
  proofB64: "cHJvb2Y=",
});

if (env.execute.msg.register_claim.issuer !== "terp.network/passkey-zkjwt") {
  throw new Error("register_claim plane broken");
}
if (env.saTx.addAuthenticator !== AUTH_TYPE) throw new Error("wrong authenticator type");
if (env.saTx.selectedAuthenticatorData.circuit_id !== CIRCUIT_ID) {
  throw new Error("circuit_id must be flock.v1");
}
assertNotExecuteAuthenticate(env.execute.msg);
try {
  assertNotExecuteAuthenticate({ authenticate: {} });
  throw new Error("should reject execute authenticate");
} catch (e) {
  if (!(e instanceof Error) || !e.message.includes("Authenticate is gone")) throw e;
}

const signed = signableFromEnvelope("terp1alice", env);
if (signed.registerClaim.typeUrl !== MSG_EXECUTE) throw new Error("wrong typeUrl");
const decoded = JSON.parse(new TextDecoder().decode(signed.registerClaim.value.msg));
if (!decoded.register_claim) throw new Error("execute must be register_claim");
if (decoded.authenticate) throw new Error("execute authenticate leaked");
const ante = JSON.parse(new TextDecoder().decode(signed.saAuth.selectedAuthenticatorData));
if (ante.circuit_id !== CIRCUIT_ID) throw new Error("ante payload circuit_id");
if (ante.proof !== "cHJvb2Y=") throw new Error("ante payload proof");

const { signAndBroadcast } = await import("./sign.ts");
const txs: string[] = [];
const hashes = await signAndBroadcast(
  {
    address: "terp1alice",
    async broadcastExecute(msg) {
      const m = JSON.parse(new TextDecoder().decode(msg.value.msg));
      if (m.authenticate) throw new Error("signer saw execute authenticate");
      if (!m.register_claim) throw new Error("expected register_claim");
      txs.push("claim");
      return { hash: "0xclaim", kind: "register_claim" };
    },
    async broadcastSaTx(auth) {
      if (auth.authenticatorType !== AUTH_TYPE) throw new Error("auth type");
      txs.push("sa");
      return { hash: "0xsa", kind: "sa_auth" };
    },
  },
  env,
);
if (txs.join(",") !== "claim,sa") throw new Error("order must be register_claim then SA");
if (hashes.length !== 2) throw new Error("two txs");

console.log("g3-fe-envelope: ok");
