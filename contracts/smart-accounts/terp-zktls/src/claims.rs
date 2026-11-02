#![allow(non_snake_case)]

use crate::{
    error::ContractError,
    state::{Epoch, Witness},
};
use cosmwasm_schema::cw_serde;
use sha2::Sha256;
use sha3::{Digest, Keccak256};

use cosmwasm_std::{DepsMut, StdError};

#[cw_serde]
pub struct ProofMsg {
    pub proof: Proof,
}

#[cw_serde]
#[serde(rename_all = "snake_case")]
pub struct ClaimInfo {
    pub provider: String,
    pub parameters: String,
    pub context: String,
}

#[cw_serde]
pub struct CompleteClaimData {
    pub identifier: String,
    pub owner: String,
    pub epoch: u64,
    pub timestampS: u64,
}

#[cw_serde]
#[serde(rename_all = "snake_case")]
pub struct SignedClaim {
    pub claim: CompleteClaimData,
    pub signatures: Vec<String>,
}

pub fn append_0x(content: &str) -> String {
    let mut initializer = String::from("0x");
    initializer.push_str(content);
    initializer
}

pub fn keccak256(message: &str) -> Vec<u8> {
    let message: &[u8] = message.as_ref();

    let mut eth_message = format!("\x19Ethereum Signed Message:\n{}", message.len()).into_bytes();
    eth_message.extend_from_slice(message);
    let mut hasher = Keccak256::new();
    hasher.update(&eth_message);

    hasher.finalize().to_vec()
}

fn generate_random_seed(bytes: Vec<u8>, offset: usize) -> u32 {
    // Convert the hash result into a u32 using the offset
    let hash_slice = &bytes[offset..offset + 4];
    let mut seed = 0u32;
    for (i, &byte) in hash_slice.iter().enumerate() {
        seed |= u32::from(byte) << (i * 8);
    }

    seed
}

pub fn fetch_witness_for_claim(
    epoch: Epoch,
    identifier: String,
    timestamp: cosmwasm_std::Timestamp,
) -> Vec<Witness> {
    let mut selected_witness = vec![];

    // Create a hash from identifier+epoch+minimum+timestamp
    let hash_str = format!(
        "{}\n{}\n{}\n{}",
        hex::encode(identifier),
        epoch.minimum_witness_for_claim_creation,
        timestamp.nanos(),
        epoch.id
    );
    let result = hash_str.as_bytes().to_vec();
    let hash_result: [u8; 32] = Sha256::digest(&result).to_vec().try_into().unwrap();

    let witenesses_left_list = epoch.witness;
    let mut byte_offset = 0;
    let witness_left = witenesses_left_list.len();
    for _i in 0..epoch.minimum_witness_for_claim_creation.into() {
        let random_seed = generate_random_seed(hash_result.to_vec(), byte_offset) as usize;
        let witness_index = random_seed % witness_left;
        let witness = witenesses_left_list.get(witness_index);
        if let Some(data) = witness {
            selected_witness.push(data.clone())
        }
        byte_offset = (byte_offset + 4) % hash_result.len();
    }

    selected_witness
}

impl ClaimInfo {
    pub fn hash(&self) -> String {
        let mut hasher = Keccak256::new();
        let hash_str = format!(
            "{}\n{}\n{}",
            &self.provider, &self.parameters, &self.context
        );
        hasher.update(&hash_str);

        let hash = hasher.finalize().to_vec();
        append_0x(hex::encode(hash).as_str())
    }
}

impl CompleteClaimData {
    pub fn serialise(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}",
            &self.identifier,
            &self.owner.to_string(),
            &self.timestampS.to_string(),
            &self.epoch.to_string()
        )
    }
}

impl SignedClaim {
    pub fn recover_signers_of_signed_claim(
        self,
        deps: DepsMut,
    ) -> Result<Vec<String>, ContractError> {
        let mut expected = vec![];
        let serialised_claim = self.claim.serialise();
        let bm = keccak256(serialised_claim.as_str());
        let message_hash: [u8; 32] = bm
            .as_slice()
            .try_into()
            .map_err(|_| ContractError::Std(StdError::msg("keccak digest is not 32 bytes")))?;

        for mut complete_signature in self.signatures {
            complete_signature.remove(0);
            complete_signature.remove(0);
            let rec_param = complete_signature
                .get((complete_signature.len() - 2)..complete_signature.len())
                .unwrap();
            let mut mut_sig_str = complete_signature.clone();
            mut_sig_str.pop();
            mut_sig_str.pop();

            let rec_dec = hex::decode(rec_param).unwrap();
            let rec_norm = rec_dec.first().unwrap() - 27;
            if rec_norm > 1 {
                return Err(ContractError::Std(StdError::msg("ayo lets go")));
            }
            let r_s = hex::decode(mut_sig_str).unwrap();
            if r_s.len() != 64 {
                return Err(ContractError::Std(StdError::msg(
                    "signature must be 64 bytes",
                )));
            }

            let pubkey = deps
                .api
                .secp256k1_recover_pubkey(&message_hash, &r_s, rec_norm)
                .map_err(|e| {
                    ContractError::Std(StdError::msg(format!("secp256k1_recover_pubkey: {e}")))
                })?;
            let key = if pubkey.len() == 65 && pubkey[0] == 0x04 {
                &pubkey[1..]
            } else {
                pubkey.as_slice()
            };
            let hasher = Keccak256::new_with_prefix(key);

            let hash = hasher.finalize().to_vec();

            let address_bytes = hash.get(12..).unwrap();
            let public_key = append_0x(&hex::encode(address_bytes));
            expected.push(public_key);
        }
        Ok(expected)
    }
}

#[cw_serde]
#[serde(rename_all = "snake_case")]
pub struct Proof {
    pub claimInfo: ClaimInfo,
    pub signedClaim: SignedClaim,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::mock_dependencies;
    use k256::ecdsa::SigningKey;

    #[test]
    fn host_recover_matches_ethereum_address() {
        let sk = SigningKey::from_slice(&[0x11u8; 32]).unwrap();
        let claim = CompleteClaimData {
            identifier: "id".into(),
            owner: "owner".into(),
            epoch: 1,
            timestampS: 2,
        };
        let hash = keccak256(&claim.serialise());
        let (sig, recid) = sk.sign_prehash_recoverable(&hash);
        let mut body = hex::encode(sig.to_bytes());
        body.push_str(&hex::encode([recid.to_byte() + 27]));
        let signed = SignedClaim {
            claim,
            signatures: vec![format!("0x{body}")],
        };
        let vk = sk.verifying_key().to_sec1_point(false);
        let addr_hash = Keccak256::digest(&vk.as_bytes()[1..]);
        let expect = append_0x(&hex::encode(&addr_hash[12..]));

        let mut deps = mock_dependencies();
        let got = signed
            .recover_signers_of_signed_claim(deps.as_mut())
            .unwrap();
        assert_eq!(got, vec![expect]);
    }
}
