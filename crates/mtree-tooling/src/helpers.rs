use blake3::Hash;
use cosmwasm_std::{Addr, Api, HexBinary, StdError, StdResult};
use url::Url;

pub fn valid_hash_string(hash_string: &String) -> StdResult<()> {
    let hex_res = HexBinary::from_hex(hash_string.as_str());
    if hex_res.is_err() {
        return Err(cosmwasm_std::StdError::msg(format!(
            "invalid hash {}",
            hash_string
        )));
    }

    let hex_binary = hex_res.unwrap();

    let decoded = hex_binary.to_array::<32>();

    if decoded.is_err() {
        return Err(cosmwasm_std::StdError::msg("invalid hash length"));
    }
    Ok(())
}

pub fn verify_merkle_root(merkle_root: &String) -> StdResult<()> {
    valid_hash_string(merkle_root)
}

pub fn string_to_hash(string: &String) -> StdResult<Hash> {
    let mut byte_slice = [0; 32];
    hex::decode_to_slice(string, &mut byte_slice)
        .map_err(|_| StdError::msg("Couldn't decode hash string".to_string()))?;
    Ok(Hash::from_bytes(byte_slice))
}

pub fn verify_tree_uri(tree_uri: &Option<String>) -> StdResult<()> {
    if tree_uri.is_some() {
        let res = Url::parse(tree_uri.as_ref().unwrap());
        if res.is_err() {
            return Err(cosmwasm_std::StdError::msg(
                "Invalid tree uri".to_string(),
            ));
        }
    }
    Ok(())
}

pub fn map_validate(api: &dyn Api, admins: &[String]) -> StdResult<Vec<Addr>> {
    admins.iter().map(|addr| api.addr_validate(addr)).collect()
}
