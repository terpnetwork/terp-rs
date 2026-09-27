//! Instantiate + sudo Authenticate matrix (no other authenticators).

#[cfg(test)]
mod tests {
    use cosmwasm_std::{
        testing::{mock_dependencies, mock_env},
        to_json_binary, Addr, Binary, MessageInfo,
    };

    use crate::msg::{
        CircuitVerifyingKeys, ExecuteMsg, InstantiateMsg, VsckAuthPayload, VsckCircuit, VsckRole,
    };
    use crate::{execute, instantiate, sudo};

    fn vks() -> CircuitVerifyingKeys {
        CircuitVerifyingKeys {
            delegation_vk: Binary::from(b"del-vk"),
            vote_proof_vk: Binary::from(b"vote-vk"),
            share_reveal_vk: Binary::from(b"share-vk"),
        }
    }

    fn root() -> Binary {
        Binary::from(b"note-tree-root-32bytes___________")
    }

    fn payload(session: &str, role: VsckRole, circuit: VsckCircuit) -> VsckAuthPayload {
        let mut pi = root().to_vec();
        pi.extend_from_slice(b"|ballot");
        VsckAuthPayload {
            session_id: session.into(),
            role,
            circuit,
            public_inputs: Binary::from(pi),
            proof: Binary::from(b"proof-bytes"),
            nullifier: Binary::from(b"n"),
        }
    }

    fn auth_sudo(p: VsckAuthPayload) -> crate::SudoMsg {
        let acct = Addr::unchecked("acct");
        crate::SudoMsg::Authenticate(Box::new(terp_auth::AuthenticationRequest {
            authenticator_id: "1".into(),
            account: acct.clone(),
            fee_payer: acct.clone(),
            fee_granter: None,
            fee: vec![],
            msg: terp_auth::Any {
                type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
                value: Binary::default(),
            },
            msg_index: 0,
            signature: to_json_binary(&p).unwrap(),
            sign_mode_tx_data: terp_auth::SignModeTxData {
                sign_mode_direct: Binary::default(),
                sign_mode_textual: None,
            },
            tx_data: terp_auth::TxData {
                chain_id: "terp-test-1".into(),
                account_number: 0,
                sequence: 0,
                timeout_height: 0,
                msgs: vec![],
                memo: String::new(),
            },
            signature_data: terp_auth::SignatureData {
                signers: vec![acct],
                signatures: vec![],
            },
            simulate: false,
            authenticator_params: None,
        }))
    }

    fn open_and_auth(session: &str, p: VsckAuthPayload) -> Result<cosmwasm_std::Response, String> {
        let mut deps = mock_dependencies();
        let admin = deps.api.addr_make("admin");
        let dao = deps.api.addr_make("dao");
        instantiate(
            deps.as_mut(),
            mock_env(),
            MessageInfo {
                sender: admin.clone(),
                funds: vec![],
            },
            InstantiateMsg {
                admin: Some(admin.to_string()),
                dao: dao.to_string(),
                circuit_vks: vks(),
            },
        )
        .map_err(|e| format!("instantiate: {e}"))?;
        execute(
            deps.as_mut(),
            mock_env(),
            MessageInfo {
                sender: admin,
                funds: vec![],
            },
            ExecuteMsg::OpenSession {
                session_id: session.into(),
                note_tree_root: root(),
                proposal_ref: Some("p".into()),
            },
        )
        .map_err(|e| format!("execute: {e}"))?;
        sudo(deps.as_mut(), mock_env(), auth_sudo(p)).map_err(|e| format!("sudo: {e}"))
    }

    #[test]
    fn sudo_delegation_fail_closed() {
        let e = open_and_auth(
            "d",
            payload("d", VsckRole::Delegator, VsckCircuit::Delegation),
        )
        .unwrap_err();
        assert!(e.contains("not linked"), "{e}");
    }

    #[test]
    fn sudo_vote_proof_fail_closed() {
        let e = open_and_auth("v", payload("v", VsckRole::Voter, VsckCircuit::VoteProof))
            .unwrap_err();
        assert!(e.contains("not linked"), "{e}");
    }

    #[test]
    fn sudo_share_reveal_fail_closed() {
        let e = open_and_auth(
            "s",
            payload("s", VsckRole::Tallier, VsckCircuit::ShareReveal),
        )
        .unwrap_err();
        assert!(e.contains("not linked"), "{e}");
    }

    #[test]
    fn sudo_coordinator_denied() {
        let e = open_and_auth(
            "c",
            payload("c", VsckRole::Coordinator, VsckCircuit::VoteProof),
        )
        .unwrap_err();
        assert!(e.contains("Coordinator"), "{e}");
    }

    #[test]
    fn sudo_unbound_public_inputs() {
        let e = open_and_auth(
            "u",
            VsckAuthPayload {
                session_id: "u".into(),
                role: VsckRole::Voter,
                circuit: VsckCircuit::VoteProof,
                public_inputs: Binary::from(b"wrong"),
                proof: Binary::from(b"proof"),
                nullifier: Binary::from(b"n"),
            },
        )
        .unwrap_err();
        assert!(e.contains("note_tree_root"), "{e}");
    }
}
