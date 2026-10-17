//! cw-orch suite helpers for `cw-private-dex` (CreatePool / SettleSwap / queries).
//!
//! G3 harness settle. SSOT: `DESIGN-HARNESS-SETTLE.md`.

use cosmwasm_std::{Binary, Uint128};
use cw_orch::prelude::*;
use cw_private_dex::interface::CwPrivateDexContract;
use cw_private_dex::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, Pool, QueryMsg, QuoteResponse, SwapStatementPublic,
};

use crate::harness::swap_statement_cw::{
    lab_allowed_root, LAB_GAMMA, LAB_GAMMA_DEN, LAB_R_IN, LAB_R_OUT, SwapSpendHandoffV0,
};

/// Thin suite around one `cw-private-dex` instance.
pub struct PrivateDexSuite<Chain: CwEnv> {
    pub dex: CwPrivateDexContract<Chain>,
}

impl<Chain: CwEnv> PrivateDexSuite<Chain> {
    pub fn new(chain: Chain) -> Self
    where
        Chain: Clone,
    {
        Self {
            dex: CwPrivateDexContract::new(chain),
        }
    }

    /// Upload + instantiate with lab mock_verify and allowed root window.
    pub fn upload_and_instantiate(
        &mut self,
        mock_verify: bool,
        allowed_root: Option<Binary>,
    ) -> Result<(), CwOrchError> {
        self.dex.upload()?;
        self.dex.instantiate(
            &InstantiateMsg {
                mock_verify,
                zkid: None,
                allowed_root,
            },
            None,
            &[],
        )?;
        Ok(())
    }

    /// Lab defaults: mock_verify + fixed root `[7;32]`.
    pub fn upload_and_instantiate_lab(&mut self, mock_verify: bool) -> Result<(), CwOrchError> {
        self.upload_and_instantiate(
            mock_verify,
            Some(Binary::from(lab_allowed_root().to_vec())),
        )
    }

    pub fn create_pool(
        &self,
        asset_a: Binary,
        asset_b: Binary,
        r_a: u128,
        r_b: u128,
        gamma: u64,
        gamma_den: u64,
    ) -> Result<<Chain as TxHandler>::Response, CwOrchError> {
        self.dex.execute(
            &ExecuteMsg::CreatePool {
                asset_a,
                asset_b,
                r_a: Uint128::new(r_a),
                r_b: Uint128::new(r_b),
                gamma,
                gamma_den,
            },
            &[],
        )
    }

    /// Create lab pool for handoff asset orientation (asset_a = in, asset_b = out).
    ///
    /// **Magic reserves** (`LAB_R_IN`/`LAB_R_OUT`) — structure demos only.
    /// Omni product path: use [`Self::create_pool_from_lp_seed`] after
    /// `bootstrap_omni_liquidity` (GUIDE Phases 1–3).
    pub fn create_lab_pool_for_handoff(
        &self,
        handoff: &SwapSpendHandoffV0,
    ) -> Result<<Chain as TxHandler>::Response, CwOrchError> {
        self.create_pool(
            handoff.statement.asset_in.clone(),
            handoff.statement.asset_out.clone(),
            LAB_R_IN,
            LAB_R_OUT,
            LAB_GAMMA,
            LAB_GAMMA_DEN,
        )
    }

    /// Create pool with reserves from bridged LP seed receipt (Phase 3 lab CW equivalent).
    ///
    /// `r_a`/`r_b` **must** equal LP note values recorded in the receipt; do not invent R.
    pub fn create_pool_from_lp_seed(
        &self,
        handoff: &SwapSpendHandoffV0,
        r_btc: u128,
        r_zec: u128,
    ) -> Result<<Chain as TxHandler>::Response, CwOrchError> {
        self.create_pool(
            handoff.statement.asset_in.clone(),
            handoff.statement.asset_out.clone(),
            r_btc,
            r_zec,
            LAB_GAMMA,
            LAB_GAMMA_DEN,
        )
    }

    pub fn settle_swap(
        &self,
        statement: SwapStatementPublic,
        proof: Binary,
    ) -> Result<<Chain as TxHandler>::Response, CwOrchError> {
        self.dex.execute(
            &ExecuteMsg::SettleSwap { statement, proof },
            &[],
        )
    }

    pub fn settle_handoff(
        &self,
        handoff: &SwapSpendHandoffV0,
    ) -> Result<<Chain as TxHandler>::Response, CwOrchError> {
        self.settle_swap(handoff.statement.clone(), handoff.proof.clone())
    }

    pub fn query_config(&self) -> Result<ConfigResponse, CwOrchError> {
        self.dex.query(&QueryMsg::Config {})
    }

    pub fn query_pool(&self, pool_id: u64) -> Result<Pool, CwOrchError> {
        self.dex.query(&QueryMsg::Pool { pool_id })
    }

    pub fn query_nullifier_spent(&self, nullifier: Binary) -> Result<bool, CwOrchError> {
        self.dex.query(&QueryMsg::IsNullifierSpent { nullifier })
    }

    pub fn query_quote(
        &self,
        pool_id: u64,
        asset_in: Binary,
        delta_in: u128,
    ) -> Result<QuoteResponse, CwOrchError> {
        self.dex.query(&QueryMsg::QuoteExactIn {
            pool_id,
            asset_in,
            delta_in: Uint128::new(delta_in),
        })
    }

    /// Post-settle asserts: reserves + nullifiers (fail-closed).
    pub fn assert_settle_ok(&self, handoff: &SwapSpendHandoffV0) -> Result<(u128, u128), CwOrchError> {
        let st = &handoff.statement;
        let pool = self.query_pool(st.pool_id)?;
        let r_in_after = st.r_in_before.u128() + st.delta_r_in.u128();
        let r_out_after = st.r_out_before.u128() - st.delta_r_out.u128();
        // Orient: asset_in is a (lab create puts in as a)
        if pool.r_a.u128() != r_in_after || pool.r_b.u128() != r_out_after {
            return Err(CwOrchError::StdErr(format!(
                "reserve mismatch: pool r_a={} r_b={} expected r_in_after={} r_out_after={}",
                pool.r_a, pool.r_b, r_in_after, r_out_after
            )));
        }
        for nf in &st.nullifiers {
            let spent = self.query_nullifier_spent(nf.clone())?;
            if !spent {
                return Err(CwOrchError::StdErr(format!(
                    "nullifier not spent: {}",
                    hex::encode(nf.as_slice())
                )));
            }
        }
        Ok((r_in_after, r_out_after))
    }

    pub fn address_string(&self) -> Result<String, CwOrchError> {
        Ok(self.dex.address()?.to_string())
    }

    /// Resume-friendly store: skip upload when `cw_private_dex` code_id is in state.
    pub fn upload_if_needed(&self) -> Result<(), CwOrchError> {
        self.dex.upload_if_needed()?;
        Ok(())
    }

    pub fn bound_code_id(&self) -> Option<u64> {
        self.dex.code_id().ok()
    }
}

impl<Chain: CwEnv> cw_orch::contract::Deploy<Chain> for PrivateDexSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = ();

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = Self::new(chain);
        suite.upload_if_needed()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![Box::new(&mut self.dex)]
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }

    fn deploy_on(chain: Chain, _data: Self::DeployData) -> Result<Self, Self::Error> {
        // Upload-if-needed only. Instantiation stays explicit (admin None).
        Self::store_on(chain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::mint_evidence::mint_evidence_from_claim_fields;
    use crate::harness::swap_statement_cw::{
        build_swap_spend_handoff_from_mint, ProofModeLabel,
    };

    fn fixture_mint() -> crate::harness::MintEvidenceV0 {
        let mut asset_in = [0u8; 32];
        asset_in[0] = b'H';
        asset_in[1] = b'U';
        asset_in[2] = b'B';
        mint_evidence_from_claim_fields(
            "ict_local_funded",
            "mock",
            "terp1hs",
            &[0x11; 32],
            &[0x22; 32],
            5_000,
            &asset_in,
            Some(&[0x33; 32]),
            Some(&[0x44; 32]),
            true,
            None,
            None,
        )
    }

    #[test]
    fn settle_after_mint_mock() {
        let app = Mock::new("owner");
        let mut suite = PrivateDexSuite::new(app.clone());
        suite
            .upload_and_instantiate_lab(true)
            .expect("upload/instantiate");

        let mint = fixture_mint();
        let handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();

        suite
            .create_lab_pool_for_handoff(&handoff)
            .expect("create pool");

        // Optional preflight quote
        let q = suite
            .query_quote(
                handoff.statement.pool_id,
                handoff.statement.asset_in.clone(),
                handoff.statement.delta_r_in.u128(),
            )
            .expect("quote");
        assert_eq!(q.delta_out, handoff.statement.delta_r_out);

        suite.settle_handoff(&handoff).expect("settle");
        let (r_in, r_out) = suite.assert_settle_ok(&handoff).expect("assert");
        assert_eq!(r_in, LAB_R_IN + handoff.statement.delta_r_in.u128());
        assert_eq!(r_out, LAB_R_OUT - handoff.statement.delta_r_out.u128());

        // Double settle same ν → reject
        let err = suite.settle_handoff(&handoff).unwrap_err();
        let es = err.to_string().to_lowercase();
        assert!(
            es.contains("nullifier"),
            "expected nullifier reject, got {err}"
        );

        let cfg = suite.query_config().unwrap();
        assert!(cfg.mock_verify);
    }

    #[test]
    fn empty_proof_rejected() {
        let app = Mock::new("owner");
        let mut suite = PrivateDexSuite::new(app);
        suite.upload_and_instantiate_lab(true).unwrap();
        let mint = fixture_mint();
        let mut handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();
        suite.create_lab_pool_for_handoff(&handoff).unwrap();
        handoff.proof = Binary::from(Vec::<u8>::new());
        let err = suite.settle_handoff(&handoff).unwrap_err();
        let es = err.to_string().to_lowercase();
        assert!(
            es.contains("proof") || es.contains("reject"),
            "expected proof reject, got {err}"
        );
    }

    #[test]
    fn settle_after_omni_lp_seed_reserves() {
        use crate::harness::omni_lp_seed::bootstrap_omni_liquidity;
        use crate::harness::swap_statement_cw::build_swap_spend_handoff_from_mint_with_reserves;

        let boot = bootstrap_omni_liquidity().expect("lp bootstrap");
        boot.assert_product_green().unwrap();

        let app = Mock::new("owner");
        let mut suite = PrivateDexSuite::new(app.clone());
        suite
            .upload_and_instantiate_lab(true)
            .expect("upload/instantiate");

        let mint = fixture_mint();
        let handoff = build_swap_spend_handoff_from_mint_with_reserves(
            &mint,
            ProofModeLabel::MockVerifyLab,
            boot.r_btc,
            boot.r_zec,
        )
        .unwrap();

        suite
            .create_pool_from_lp_seed(&handoff, boot.r_btc, boot.r_zec)
            .expect("create pool from bridged LP");

        let pool = suite.query_pool(handoff.statement.pool_id).unwrap();
        assert_eq!(pool.r_a.u128(), boot.r_btc);
        assert_eq!(pool.r_b.u128(), boot.r_zec);
        assert_eq!(
            handoff.statement.r_in_before.u128(),
            boot.r_btc,
            "statement binds LP-seeded R"
        );

        suite.settle_handoff(&handoff).expect("settle vs LP pool");
        let (r_in, r_out) = suite.assert_settle_ok(&handoff).expect("assert");
        assert_eq!(r_in, boot.r_btc + handoff.statement.delta_r_in.u128());
        assert_eq!(r_out, boot.r_zec - handoff.statement.delta_r_out.u128());
    }
}
