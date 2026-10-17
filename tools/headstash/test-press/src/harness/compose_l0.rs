//! Thin L0 re-export of the compose product pure path.
//!
//! SSOT implementation: `docs/plans/spectrum/fixtures/compose_seams`
//! (`run_product_path_burn_to_swap_sketch`, oracle+ZEC path, G2 builders).
//!
//! One module for harness CI: `cargo test -p zk-test-press --lib harness::compose_l0`.

use terp_seams::compose::{
    run_product_path_burn_to_swap_oracle_zec, run_product_path_burn_to_swap_sketch,
    ProductPathOracleZecOutcome, ProductPathOutcome,
};

use super::bridge_l0::L0Error;

/// Product pure E2E via compose_seams (register → mint note → swap apply).
pub fn assert_product_path_burn_to_swap_sketch() -> Result<ProductPathOutcome, L0Error> {
    run_product_path_burn_to_swap_sketch().map_err(|e| L0Error(format!("{e:?}")))
}

/// G2 product path: oracle required + corridor ZEC `asset_out`.
pub fn assert_product_path_burn_to_swap_oracle_zec() -> Result<ProductPathOracleZecOutcome, L0Error>
{
    run_product_path_burn_to_swap_oracle_zec().map_err(|e| L0Error(format!("{e:?}")))
}

// Re-export G2 types for harness consumers.
pub use terp_seams::compose::{
    mint_evidence_to_swap_action, seam_note_out_to_swap_action, seam_note_out_to_swap_openings,
    swap_action_public_to_statement, CorridorSwapSpendParams, MintSpendEvidence,
    SwapStatementPublicView,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_path_burn_to_swap_sketch() {
        let out = assert_product_path_burn_to_swap_sketch().expect("product pure e2e");
        assert_eq!(out.notes_in, 1);
        assert_eq!(out.pool_nullifiers.len(), 1);
        assert_eq!(out.r_in_after, out.r_in_before + out.delta_in);
        assert_eq!(out.r_out_after, out.r_out_before - out.delta_out);
    }

    #[test]
    fn product_path_burn_to_swap_oracle_zec() {
        let out = assert_product_path_burn_to_swap_oracle_zec().expect("oracle+zec");
        assert_eq!(out.base.notes_in, 1);
        assert_eq!(
            out.action.witness.notes_in[0].cm_public,
            out.mint_note.cm_public
        );
        assert!(out
            .action
            .public
            .oracle_params
            .as_ref()
            .map(|p| p.require_oracle)
            .unwrap_or(false));
        assert_eq!(out.statement.asset_out.len(), 32);
    }
}
