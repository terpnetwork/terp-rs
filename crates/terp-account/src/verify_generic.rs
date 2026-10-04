// condensed from: https://github.com/MegaRockLabs/smart-account-auth
use cosmwasm_std::{ensure, Api, Binary, StdError};

use bech32::{Bech32, Hrp};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

#[cosmwasm_schema::cw_serde]
pub struct CosmosArbitrary {
    pub pubkey: Binary,
    pub signature: Binary,
    pub message: Binary,
    pub hrp: Option<String>,
}

// used for testing only.
#[cosmwasm_schema::cw_serde]
pub struct TestCosmosArb {
    pub carb: CosmosArbitrary,
    // ecdsa::SigningKey binary
    pub pk: Binary,
}

impl CosmosArbitrary {
    // Before returning the sha256sum hash, take the human readable address signing this and add it along with the data into its adr036 template object being hashed.
    pub fn message_digest(&self) -> Result<Vec<u8>, StdError> {
        ensure!(
            self.hrp.is_some(),
            StdError::msg("Must provide prefix for the public key".to_string())
        );
        Ok(sha256(
            preamble_msg_arb_036(
                pubkey_to_address(&self.pubkey, self.hrp.as_ref().unwrap())?.as_str(),
                &self.message.to_string(),
            )
            .as_bytes(),
        ))
    }
    // return the human readable prefix
    pub fn hrp(&self) -> Option<String> {
        self.hrp.clone()
    }

    // generic object validations
    fn _validate(&self) -> Result<(), StdError> {
        if !(!self.signature.is_empty()
            && !self.message.to_string().is_empty()
            && !self.pubkey.is_empty())
        {
            return Err(StdError::msg("Empty credential data".to_string()));
        }
        Ok(())
    }

    // verify an arbitrary secp256k1 signature is valid.
    pub fn verify(&self, api: &dyn Api) -> Result<(), StdError> {
        let success = api
            .secp256k1_verify(&self.message_digest()?, &self.signature, &self.pubkey)
            .map_err(|e| StdError::msg(e.to_string()))?;
        ensure!(
            success,
            StdError::msg("Signature verification failed".to_string())
        );
        Ok(())
    }

    pub fn verify_return_readable(&self, api: &dyn Api) -> Result<String, StdError> {
        self.verify(api)?;
        pubkey_to_address(&self.pubkey, &self.hrp().expect("must have prefix"))
    }

    fn get_address(&self) -> Result<String, StdError> {
        pubkey_to_address(&self.pubkey, &self.hrp.clone().unwrap_or("terp".into()))
    }
}

/// inject the data to be signed within the json struct
pub fn preamble_msg_arb_036(signer: &str, data: &str) -> String {
    format!(
        "{{\"account_number\":\"0\",\"chain_id\":\"\",\"fee\":{{\"amount\":[],\"gas\":\"0\"}},\"memo\":\"\",\"msgs\":[{{\"type\":\"sign/MsgSignData\",\"value\":{{\"data\":\"{}\",\"signer\":\"{}\"}}}}],\"sequence\":\"0\"}}", 
        data, signer
    )
}

pub fn pubkey_to_address(pubkey: &[u8], hrp: &str) -> Result<String, StdError> {
    let base32_addr = ripemd160(&sha256(pubkey));
    let account: String = bech32::encode::<Bech32>(
        Hrp::parse(hrp).map_err(|e| StdError::msg(e.to_string()))?,
        &base32_addr,
    )
    .map_err(|e| StdError::msg(e.to_string()))?;
    Ok(account)
}

pub fn sha256(msg: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(msg);
    hasher.finalize().to_vec()
}

pub fn ripemd160(bytes: &[u8]) -> Vec<u8> {
    let mut hasher = Ripemd160::new();
    hasher.update(bytes);
    hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::StdError;
    use cosmwasm_std::{testing::mock_dependencies, Binary};

    use ecdsa::signature::rand_core::OsRng;

    use k256::ecdsa::{Signature, SigningKey, VerifyingKey};
    // use serde::Deserialize;
    use sha2::digest::Update;
    use sha2::Digest;
    use sha2::Sha256;

    use crate::verify_generic::{preamble_msg_arb_036, pubkey_to_address, CosmosArbitrary};

    // "Cosmos" secp256k1 signature verification. Matches tendermint/PubKeySecp256k1 pubkey.
    // const COSMOS_SECP256K1_PUBKEY_HEX: &str =
    //     "034f04181eeba35391b858633a765c4a0c189697b40d216354d50890d350c70290";
    // const MSG: &str = "Hello World!";

    #[test]
    fn test_example() -> Result<(), StdError> {
        // Signing
        let secret_key: ecdsa::SigningKey<k256::Secp256k1> = SigningKey::random(&mut OsRng); // Serialize with `::to_bytes()`
        let public_key: ecdsa::VerifyingKey<k256::Secp256k1> = VerifyingKey::from(&secret_key); // Serialize with `::to_encoded_point()`

        let deps = mock_dependencies();
        let terpaddr = deps.api.with_prefix("terp").addr_make("jablerert");
        // base64 encode address (this is the expected value in the data object of the adr036)
        let base64terpaddr = &Binary::new(terpaddr.as_bytes().to_vec()).to_base64();
        // this is the human readable address derived from the public key of the secret key
        let hraddr = pubkey_to_address(public_key.to_encoded_point(false).as_bytes(), "cosmos")?;

        // create adr036 msgs data to sign
        let adr036msgtohash = preamble_msg_arb_036(&hraddr.to_string(), base64terpaddr);
        // Explicit / external hashing
        // sha256 hash msgs data
        let msg_digest = Sha256::new().chain(&adr036msgtohash);
        let msg_hash = msg_digest.clone().finalize();

        // Note: the signature type must be annotated or otherwise inferable as
        // `Signer` has many impls of the `Signer` trait (for both regular and
        // recoverable signature types).
        let signature: Signature = secret_key.sign_prehash_recoverable(&msg_hash).unwrap().0;

        // Verification (uncompressed public key)
        assert!(cosmwasm_crypto::secp256k1_verify(
            &msg_hash,
            signature.to_bytes().as_slice(),
            public_key.to_encoded_point(false).as_bytes()
        )
        .unwrap());

        // Verification (compressed public key)
        assert!(cosmwasm_crypto::secp256k1_verify(
            &msg_hash,
            signature.to_bytes().as_slice(),
            public_key.to_encoded_point(true).as_bytes()
        )
        .unwrap());

        let hrp = "cosmos";

        // CosmosArbitrary Verification (compressed public key)
        CosmosArbitrary {
            pubkey: Binary::from(public_key.to_encoded_point(false).as_bytes()),
            signature: Binary::from(signature.to_bytes().as_slice()),
            message: Binary::from(terpaddr.as_bytes().to_vec()), // set the base64 of the address
            hrp: Some(hrp.to_string()),
        }
        .verify_return_readable(&cosmwasm_std::testing::mock_dependencies().api)?;

        // println!("hraddr: {:#?}", hraddr);
        // println!("terpaddr: {:#?}", terpaddr);
        // println!("base64terpaddr: {:#?}", base64terpaddr);
        // println!("adr036msgtohash: {:#?}", adr036msgtohash.to_string());
        // println!("msg_hash:  {:#?}", Binary::new(msg_hash.to_vec()));
        // println!("signature:  {:#?}", Binary::new(signature.to_vec()));
        Ok(())
    }
}
