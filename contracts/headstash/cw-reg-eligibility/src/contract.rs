use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};

use crate::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg, VerifyResponse,
};
use crate::state::{Config, CONFIG};

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    if msg.accepted_proof.is_empty() {
        return Err(cosmwasm_std::StdError::msg("accepted_proof required"));
    }
    CONFIG.save(
        deps.storage,
        &Config {
            accepted_proof: msg.accepted_proof,
            dummy_account: msg.dummy_account,
        },
    )?;
    Ok(Response::new().add_attribute("action", "instantiate_reg_eligibility"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> StdResult<Response> {
    match msg {
        ExecuteMsg::SetAcceptedProof { proof } => {
            if proof.is_empty() {
                return Err(cosmwasm_std::StdError::msg("proof required"));
            }
            CONFIG.update(deps.storage, |mut c| -> StdResult<_> {
                c.accepted_proof = proof;
                Ok(c)
            })?;
            Ok(Response::new().add_attribute("action", "set_accepted_proof"))
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let c = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                dummy_account: c.dummy_account,
                note: "lab stand-in for dummy SA wasmvm Authenticate; not L0".into(),
            })
        }
        QueryMsg::VerifyMinHolders {
            session_id: _,
            registrant: _,
            proof,
        } => {
            let c = CONFIG.load(deps.storage)?;
            to_json_binary(&VerifyResponse {
                ok: proof == c.accepted_proof,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{from_json, Addr, Binary, Coin, MessageInfo};

    fn mock_info(sender: &str, funds: &[Coin]) -> MessageInfo {
        message_info(&Addr::unchecked(sender), funds)
    }

    #[test]
    fn fixture_proof_ok_garbage_denied() {
        let mut deps = mock_dependencies();
        let ok = Binary::from(b"min-zec-holders-ok");
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("dummy", &[]),
            InstantiateMsg {
                accepted_proof: ok.clone(),
                dummy_account: Some("terp1dummy".into()),
            },
        )
        .unwrap();

        let good: VerifyResponse = from_json(
            query(
                deps.as_ref(),
                mock_env(),
                QueryMsg::VerifyMinHolders {
                    session_id: "s".into(),
                    registrant: "terp1alice".into(),
                    proof: ok,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(good.ok);

        let bad: VerifyResponse = from_json(
            query(
                deps.as_ref(),
                mock_env(),
                QueryMsg::VerifyMinHolders {
                    session_id: "s".into(),
                    registrant: "terp1alice".into(),
                    proof: Binary::from(b"nope"),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(!bad.ok);
    }
}
