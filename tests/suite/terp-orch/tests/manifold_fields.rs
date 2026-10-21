#![cfg(feature = "stack")]
//! Every TerpArtifactStack field is addressable.
//! Contract directories have their own `e2e` package that touches one field.
//! This test touches all of them in one place.

use cw_orch::prelude::*;
use terp_orch::TerpArtifactStack;

#[test]
fn every_contract_is_a_field() {
    let suite = TerpArtifactStack::new(MockBech32::new("terp"));
    let _ = (&suite.circuits.verify, &suite.dao.dao_core);
    let _ = &suite.bridge;
    let auth = &suite.authenticators;
    let _ = (
        &auth.passkey,
        &auth.recovery,
        &auth.ed25519,
        &auth.eth,
        &auth.zk_jwt,
        &auth.vsck,
    );
    let accounts = &suite.accounts;
    let _ = (
        &accounts.dao,
        &accounts.dregg,
        &accounts.frost,
        &accounts.bls,
        &accounts.zktls,
        &accounts.polytone,
    );
    let light = &suite.light_clients;
    let _ = (&light.crosslink, &light.dregg, &light.terp, &light.zap1);
    let products = &suite.products;
    let _ = (
        &products.shitstrap,
        &products.shitstrap_factory,
        &products.raffle,
        &products.loan,
        &products.marketplace,
        &products.reserve_auction,
        &products.bounty_escrow,
        &products.cw420,
        &products.ave_factory,
        &products.ave,
        &products.headstash,
        &products.manifold,
        &products.norick,
        &products.nullifier_auth,
        &products.nullifier_gate,
        &products.vote_anchor,
        &products.vote_ceremony,
        &products.reg_eligibility,
        &products.svg_auction,
        &products.oracle_aggregator,
        &products.oracle_twap_probe,
        &products.marketplace_mirror,
        &products.loyalty_verifier,
        &products.trusted_builder,
        &products.cashu_registry,
        &products.hm_nft_minter,
        &products.oracle_registry,
    );
    let infuser = &suite.infuser;
    let _ = (
        &infuser.infuser,
        &infuser.svg,
        &infuser.minter,
        &infuser.whitelist,
    );
    let boards = &suite.billboards;
    let _ = (&boards.collection, &boards.minter, &boards.ownership);
}
