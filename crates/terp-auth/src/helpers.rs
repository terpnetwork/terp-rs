use cosmwasm_std::{StdResult, to_json_binary};
use sha2::{Digest, Sha256};

use crate::{Any, MsgAddAuthenticator};

pub fn sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Register a given seckp256k1 key with a specific authenticator
pub async fn setup_terp_smart_account(authenticator: MsgAddAuthenticator) -> StdResult<Any> {
    // register custom authenticator to account
    Ok(Any {
        type_url: "/terp.smartaccount.v1beta1.MsgAddAuthenticator".into(),
        value: to_json_binary(&authenticator)?,
    })
}
