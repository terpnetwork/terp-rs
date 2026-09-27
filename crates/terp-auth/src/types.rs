use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Binary, Coin};

#[cw_serde]
pub struct AccountAuthenticator {
    /// ID uniquely identifies the authenticator instance.
    pub id: u64,
    /// Type specifies the category of the AccountAuthenticator.
    /// This type information is essential for differentiating authenticators
    /// and ensuring precise data retrieval from the storage layer.
    pub r#type: String,
    /// Config is a versatile field used in conjunction with the specific type of
    /// account authenticator to facilitate complex authentication processes.
    /// The interpretation of this field is overloaded, enabling multiple
    /// authenticators to utilize it for their respective purposes.
    pub config: Vec<u8>,
}

#[cw_serde]
pub struct OnAuthenticatorAddedRequest {
    pub account: Addr,
    pub authenticator_id: String,
    pub authenticator_params: Option<Binary>,
}
#[cw_serde]
pub struct OnAuthenticatorRemovedRequest {
    pub account: Addr,
    pub authenticator_id: String,
    pub authenticator_params: Option<Binary>,
}
#[cw_serde]
pub struct AuthenticationRequest {
    pub authenticator_id: String,
    pub account: Addr,
    pub fee_payer: Addr,
    pub fee_granter: Option<Addr>,
    pub fee: Vec<Coin>,
    pub msg: Any,
    pub msg_index: u64,
    pub signature: Binary,
    pub sign_mode_tx_data: SignModeTxData,
    pub tx_data: TxData,
    pub signature_data: SignatureData,
    pub simulate: bool,
    pub authenticator_params: Option<Binary>,
}
#[cw_serde]
pub struct TrackRequest {
    pub authenticator_id: String,
    pub account: Addr,
    pub fee_payer: Addr,
    pub fee_granter: Option<Addr>,
    pub fee: Vec<Coin>,
    pub msg: Any,
    pub msg_index: u64,
    pub authenticator_params: Option<Binary>,
}
#[cw_serde]
pub struct ConfirmExecutionRequest {
    pub authenticator_id: String,
    pub account: Addr,
    pub fee_payer: Addr,
    pub fee_granter: Option<Addr>,
    pub fee: Vec<Coin>,
    pub msg: Any,
    pub msg_index: u64,
    pub authenticator_params: Option<Binary>,
}

// --- data ---
#[cw_serde]
pub struct SignModeTxData {
    pub sign_mode_direct: Binary,
    pub sign_mode_textual: Option<String>, // Assuming it's a string or null
}
#[cw_serde]

pub struct TxData {
    pub chain_id: String,
    pub account_number: u64,
    pub sequence: u64,
    pub timeout_height: u64,
    pub msgs: Vec<Any>,
    pub memo: String,
}
#[cw_serde]
pub struct SignatureData {
    pub signers: Vec<Addr>,
    pub signatures: Vec<Binary>,
}
#[cw_serde]
pub struct Any {
    pub type_url: String,
    pub value: cosmwasm_std::Binary,
}

#[cw_serde]
pub struct MsgAddAuthenticator {
    pub sender: String,
    pub authenticator_type: String,
    pub data: Vec<u8>,
}
#[cw_serde]
pub struct CosmwasmAuthenticatorInitData {
    pub contract: String,
    pub params: Vec<u8>,
}

/// `AuthSudoMsg` contains variants of messages that can be sent to the authenticator contract
/// from smart account module through `CosmWasmAuthenticator`.
///
/// `AuthenticateRequest` is `Box`-ed due to large size difference between other variants
#[cw_serde]
pub enum AuthSudoMsg {
    /// Osmosis CosmwasmAuthenticator JSON: on_authenticator_added
    #[serde(rename = "on_authenticator_added")]
    OnAuthAdded(OnAuthenticatorAddedRequest),
    #[serde(rename = "on_authenticator_removed")]
    OnAuthRemoved(OnAuthenticatorRemovedRequest),
    Authenticate(Box<AuthenticationRequest>),
    Track(TrackRequest),
    ConfirmExecution(ConfirmExecutionRequest),
}

#[cfg(test)]
mod tests {
    use cosmwasm_schema::schemars::JsonSchema;
    use serde_json::{from_str, to_string, to_value, Value};

    use super::*;

    #[test]
    fn test_any() {
        let t = Any {
            type_url: "type_url".to_string(),
            value: Binary::from(vec![0x01, 0x02, 0x03]),
        };

        assert_eq!(t, with_unknown_field(t.clone()));
        has_json_schema_impl::<Any>();
    }

    #[test]
    fn test_on_authenticator_added_request() {
        let t = OnAuthenticatorAddedRequest {
            account: Addr::unchecked("account"),
            authenticator_id: "authenticator_id".to_string(),
            authenticator_params: Some(Binary::from(vec![0x01, 0x02, 0x03])),
        };

        assert_eq!(t, with_unknown_field(t.clone()));
        has_json_schema_impl::<OnAuthenticatorAddedRequest>();
    }

    #[test]
    fn test_on_authenticator_removed_request() {
        let t = OnAuthenticatorRemovedRequest {
            account: Addr::unchecked("account"),
            authenticator_id: "authenticator_id".to_string(),
            authenticator_params: Some(Binary::from(vec![0x01, 0x02, 0x03])),
        };

        assert_eq!(t, with_unknown_field(t.clone()));
        has_json_schema_impl::<OnAuthenticatorRemovedRequest>();
    }

    #[test]
    fn test_authentication_request() {
        let t = AuthenticationRequest {
            authenticator_id: "authenticator_id".to_string(),
            account: Addr::unchecked("account"),
            fee_payer: Addr::unchecked("fee_payer"),
            fee_granter: None,
            fee: vec![Coin::new(1000u128, "uosmo")],
            msg: Any {
                type_url: "type_url".to_string(),
                value: Binary::from(vec![0x01, 0x02, 0x03]),
            },
            msg_index: 1,
            signature: Binary::from(vec![0x01, 0x02, 0x03]),
            sign_mode_tx_data: SignModeTxData {
                sign_mode_direct: Binary::from(vec![0x01, 0x02, 0x03]),
                sign_mode_textual: Some("sign_mode_textual".to_string()),
            },
            tx_data: TxData {
                chain_id: "chain_id".to_string(),
                account_number: 1,
                sequence: 1,
                timeout_height: 1,
                msgs: vec![Any {
                    type_url: "type_url".to_string(),
                    value: Binary::from(vec![0x01, 0x02, 0x03]),
                }],
                memo: "memo".to_string(),
            },
            signature_data: SignatureData {
                signers: vec![Addr::unchecked("account")],
                signatures: vec![Binary::from(vec![0x01, 0x02, 0x03])],
            },
            simulate: true,
            authenticator_params: Some(Binary::from(vec![0x01, 0x02, 0x03])),
        };

        assert_eq!(t, with_unknown_field(t.clone()));
        has_json_schema_impl::<AuthenticationRequest>();
    }

    #[test]
    fn test_sign_mode_tx_data() {
        let t = SignModeTxData {
            sign_mode_direct: Binary::from(vec![0x01, 0x02, 0x03]),
            sign_mode_textual: Some("sign_mode_textual".to_string()),
        };

        assert_eq!(t, with_unknown_field(t.clone()));
        has_json_schema_impl::<SignModeTxData>();
    }

    #[test]
    fn test_tx_data() {
        let t = TxData {
            chain_id: "chain_id".to_string(),
            account_number: 1,
            sequence: 1,
            timeout_height: 1,
            msgs: vec![Any {
                type_url: "type_url".to_string(),
                value: Binary::from(vec![0x01, 0x02, 0x03]),
            }],
            memo: "memo".to_string(),
        };

        assert_eq!(t, with_unknown_field(t.clone()));
        has_json_schema_impl::<TxData>();
    }

    #[test]
    fn test_signature_data() {
        let t = SignatureData {
            signers: vec![Addr::unchecked("account")],
            signatures: vec![Binary::from(vec![0x01, 0x02, 0x03])],
        };

        assert_eq!(t, with_unknown_field(t.clone()));
        has_json_schema_impl::<SignatureData>();
    }

    #[test]
    fn test_track_request() {
        let t = TrackRequest {
            authenticator_id: "authenticator_id".to_string(),
            account: Addr::unchecked("account"),
            fee_payer: Addr::unchecked("fee_payer"),
            fee_granter: None,
            fee: vec![Coin::new(1000u128, "uosmo")],
            msg: Any {
                type_url: "type_url".to_string(),
                value: Binary::from(vec![0x01, 0x02, 0x03]),
            },
            msg_index: 1,
            authenticator_params: Some(Binary::from(vec![0x01, 0x02, 0x03])),
        };

        assert_eq!(t, with_unknown_field(t.clone()));
        has_json_schema_impl::<TrackRequest>();
    }

    #[test]
    fn test_confirm_execution_request() {
        let t = ConfirmExecutionRequest {
            authenticator_id: "authenticator_id".to_string(),
            account: Addr::unchecked("account"),
            fee_payer: Addr::unchecked("fee_payer"),
            fee_granter: None,
            fee: vec![Coin::new(1000u128, "uosmo")],
            msg: Any {
                type_url: "type_url".to_string(),
                value: Binary::from(vec![0x01, 0x02, 0x03]),
            },
            msg_index: 1,
            authenticator_params: Some(Binary::from(vec![0x01, 0x02, 0x03])),
        };

        assert_eq!(t, with_unknown_field(t.clone()));
        has_json_schema_impl::<ConfirmExecutionRequest>();
    }

    #[test]
    fn test_sudo_msg() {
        has_json_schema_impl::<AuthSudoMsg>();
    }

    fn with_unknown_field<
        T: cosmwasm_schema::serde::Serialize + cosmwasm_schema::serde::de::DeserializeOwned,
    >(
        t: T,
    ) -> T {
        let json = to_value(t).unwrap();

        let json = match json {
            Value::Object(mut map) => {
                map.entry("unknown")
                    .or_insert(Value::String("unknown".to_string()));

                Value::Object(map)
            }
            _ => panic!("expected object"),
        };

        let json_string = to_string(&json).unwrap();

        from_str::<T>(json_string.as_str()).unwrap()
    }

    fn has_json_schema_impl<T: JsonSchema>() {}
}
