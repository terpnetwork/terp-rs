//! Multi-cycle **private-DEX + bridge seam** routes under curated multi-chain accounts.
//!
//! Exercises load/usage paths:
//! 1. Corridor: BTC deposit-backed mint → swap → ZEC egress
//! 2. Tokenfactory-backed assets on Terp (mapped asset_ids) through multi-pair pools
//! 3. Multi-hop hub routes
//! 4. Multi-cycle rebalance (A→B then B→A)
//! 5. Bridge-mint representation of foreign assets into the anonymity set then swap
//!
//! Pure host recompute via `private_dex_seams` (no Docker). Multi-net accounts can
//! later overwrite curated addresses without changing route logic.

use terp_seams::dex::{
    apply_egress_burn, apply_swap, build_egress_burn_from_settle, quote_exact_in,
    settle_opening_from_note_fields, AssetId, DestKind, EgressSeamState, NoteIn, OracleMid,
    Pool, PoolStatus, SeamState, SwapPublic, ABSTRACT_LEAF_LABEL,
};
use sha2::{Digest, Sha256};

use super::account_curation::{
    curate_lab_accounts, AccountRegistry, AccountRole, ChainKind, TokenFactoryDenomPlan,
};

/// Demo fee params (997/1000).
pub const CYCLE_GAMMA: u64 = 997;
pub const CYCLE_GAMMA_DEN: u64 = 1000;

/// Logical seam assets for multi-cycle world (extends beyond Hub/AssetB enums with 32B ids).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SeamAsset {
    Btc,
    Zec,
    /// Tokenfactory-backed alpha (sha256 factory denom).
    TfAlpha,
    TfBeta,
    TfGamma,
}

impl SeamAsset {
    pub fn as_enum(self) -> AssetId {
        match self {
            SeamAsset::Btc => AssetId::Hub,
            SeamAsset::Zec => AssetId::AssetB,
            SeamAsset::TfAlpha => AssetId::AssetC,
            SeamAsset::TfBeta => AssetId::AssetX,
            // Reuse AssetX orientation carefully — for multi-pool we key by pair
            SeamAsset::TfGamma => AssetId::AssetX,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            SeamAsset::Btc => "BTC",
            SeamAsset::Zec => "ZEC",
            SeamAsset::TfAlpha => "TF_ALPHA",
            SeamAsset::TfBeta => "TF_BETA",
            SeamAsset::TfGamma => "TF_GAMMA",
        }
    }
}

/// World state: curated accounts + pools + nullifier state + note holdings.
pub struct MultiCycleWorld {
    pub accounts: AccountRegistry,
    pub pools: Vec<Pool>,
    pub seam: SeamState,
    pub egress: EgressSeamState,
    /// Private note balances: (role_label, asset) → value
    pub notes: Vec<NoteHolding>,
    pub cycle: u32,
}

#[derive(Clone, Debug)]
pub struct NoteHolding {
    pub owner_role: AccountRole,
    pub asset: SeamAsset,
    pub value: u128,
    pub cm: [u8; 32],
    pub rcm: [u8; 32],
    pub spent: bool,
}

#[derive(Clone, Debug)]
pub struct CycleReceipt {
    pub cycle: u32,
    pub route: String,
    pub asset_in: SeamAsset,
    pub asset_out: SeamAsset,
    pub delta_in: u128,
    pub delta_out: u128,
    pub r_in_after: u128,
    pub r_out_after: u128,
}

impl MultiCycleWorld {
    /// Seed accounts, tokenfactory plans, and virtual pools (BTC/ZEC + TF pairs).
    pub fn seed_lab() -> Self {
        let accounts = curate_lab_accounts();
        let mut pools = Vec::new();

        // BTC ↔ ZEC corridor pool (large LP inventory)
        pools.push(Pool {
            pool_id: 1,
            asset_a: AssetId::Hub,
            asset_b: AssetId::AssetB,
            r_a: 10_000_000,
            r_b: 50_000_000,
            gamma: CYCLE_GAMMA,
            gamma_den: CYCLE_GAMMA_DEN,
            status: PoolStatus::Active,
        });
        // TF alpha ↔ BTC (tokenfactory asset into corridor)
        pools.push(Pool {
            pool_id: 2,
            asset_a: AssetId::AssetC,
            asset_b: AssetId::Hub,
            r_a: 20_000_000,
            r_b: 20_000_000,
            gamma: CYCLE_GAMMA,
            gamma_den: CYCLE_GAMMA_DEN,
            status: PoolStatus::Active,
        });
        // TF alpha ↔ TF beta multi-pair
        pools.push(Pool {
            pool_id: 3,
            asset_a: AssetId::AssetC,
            asset_b: AssetId::AssetX,
            r_a: 15_000_000,
            r_b: 15_000_000,
            gamma: CYCLE_GAMMA,
            gamma_den: CYCLE_GAMMA_DEN,
            status: PoolStatus::Active,
        });

        Self {
            accounts,
            pools,
            seam: SeamState::default(),
            egress: EgressSeamState::default(),
            notes: Vec::new(),
            cycle: 0,
        }
    }

    pub fn tf_plan(&self, sub: &str) -> Option<&TokenFactoryDenomPlan> {
        self.accounts.tokenfactory_by_subdenom(sub)
    }

    fn pool(&self, pool_id: u64) -> Result<&Pool, String> {
        self.pools
            .iter()
            .find(|p| p.pool_id == pool_id)
            .ok_or_else(|| format!("pool {pool_id} missing"))
    }

    /// Bridge-mint a private note for a role (deposit/TF mint → SEAM).
    pub fn mint_note(
        &mut self,
        owner: AccountRole,
        asset: SeamAsset,
        value: u128,
        salt: u8,
    ) -> Result<usize, String> {
        if value == 0 {
            return Err("mint value 0".into());
        }
        let mut rcm = [0u8; 32];
        rcm[0] = salt;
        rcm[1] = self.cycle as u8;
        rcm[2] = owner.as_str().as_bytes().first().copied().unwrap_or(0);
        let mut h = Sha256::new();
        h.update(ABSTRACT_LEAF_LABEL);
        h.update(asset.tag().as_bytes());
        h.update(value.to_le_bytes());
        h.update(rcm);
        let d = h.finalize();
        let mut cm = [0u8; 32];
        cm.copy_from_slice(&d);
        self.notes.push(NoteHolding {
            owner_role: owner,
            asset,
            value,
            cm,
            rcm,
            spent: false,
        });
        Ok(self.notes.len() - 1)
    }

    /// Apply one swap cycle on a pool; spends first unspent note of asset_in for owner.
    pub fn swap_cycle(
        &mut self,
        owner: AccountRole,
        pool_id: u64,
        asset_in: SeamAsset,
        asset_out: SeamAsset,
        delta_in: u128,
    ) -> Result<CycleReceipt, String> {
        let note_idx = self
            .notes
            .iter()
            .position(|n| {
                n.owner_role == owner && n.asset == asset_in && !n.spent && n.value >= delta_in
            })
            .ok_or_else(|| format!("no unspent {asset_in:?} note for {owner:?}"))?;

        let note = self.notes[note_idx].clone();
        let pool = self.pool(pool_id)?.clone();
        let (r_in, r_out) = if pool.asset_a == asset_in.as_enum() && pool.asset_b == asset_out.as_enum()
        {
            (pool.r_a, pool.r_b)
        } else if pool.asset_b == asset_in.as_enum() && pool.asset_a == asset_out.as_enum() {
            (pool.r_b, pool.r_a)
        } else {
            return Err("pool legs mismatch".into());
        };

        let delta_out = quote_exact_in(r_in, r_out, delta_in, pool.gamma, pool.gamma_den)
            .map_err(|e| format!("quote: {e:?}"))?;

        // Pure-host nullifiers are u64 (SPEC pure suite); derive from cm+rcm hash.
        let mut nh = Sha256::new();
        nh.update(b"pool-nf-v0");
        nh.update(note.cm);
        nh.update(note.rcm);
        nh.update(self.cycle.to_le_bytes());
        let nd = nh.finalize();
        let nullifier = u64::from_le_bytes(nd[0..8].try_into().unwrap());

        let notes_in = [NoteIn {
            asset_id: asset_in.as_enum(),
            value: note.value,
            nullifier,
        }];
        let public = SwapPublic {
            asset_in: asset_in.as_enum(),
            asset_out: asset_out.as_enum(),
            delta_in,
            min_out: delta_out,
            nullifiers: vec![nullifier],
            oracle_mid: None,
            oracle_params: None,
            now_height: 1,
        };

        let pool_idx = self
            .pools
            .iter()
            .position(|p| p.pool_id == pool_id)
            .ok_or_else(|| format!("pool {pool_id} missing"))?;
        let out = apply_swap(
            &mut self.pools[pool_idx],
            &mut self.seam,
            &notes_in,
            &public,
        )
        .map_err(|e| format!("apply_swap: {e:?}"))?;
        assert_eq!(out, delta_out);

        self.notes[note_idx].spent = true;
        // Change note if any
        if note.value > delta_in {
            let change = note.value - delta_in;
            self.mint_note(owner, asset_in, change, 0xC0u8.wrapping_add(self.cycle as u8))?;
        }
        // Output note
        self.mint_note(owner, asset_out, delta_out, 0xD0u8.wrapping_add(self.cycle as u8))?;

        let pool = &self.pools[pool_idx];
        let (r_in_after, r_out_after) =
            if pool.asset_a == asset_in.as_enum() && pool.asset_b == asset_out.as_enum() {
                (pool.r_a, pool.r_b)
            } else {
                (pool.r_b, pool.r_a)
            };

        self.cycle += 1;
        Ok(CycleReceipt {
            cycle: self.cycle,
            route: format!(
                "{}→{}@pool{}",
                asset_in.tag(),
                asset_out.tag(),
                pool_id
            ),
            asset_in,
            asset_out,
            delta_in,
            delta_out,
            r_in_after,
            r_out_after,
        })
    }

    /// Option D egress: burn ZEC note → credit Zcash user account (lab ledger).
    pub fn egress_zec_to_user(&mut self, owner: AccountRole) -> Result<u128, String> {
        let note_idx = self
            .notes
            .iter()
            .position(|n| n.owner_role == owner && n.asset == SeamAsset::Zec && !n.spent)
            .ok_or("no ZEC note to egress")?;
        let note = self.notes[note_idx].clone();
        let dest = self
            .accounts
            .require(ChainKind::Zcash, AccountRole::User)?
            .owner_binding
            .ok_or("user zec binding")?;
        let opening = settle_opening_from_note_fields(
            {
                let mut a = [0u8; 32];
                a[0] = b'Z';
                a[1] = b'E';
                a[2] = b'C';
                a
            },
            note.value as u64,
            dest,
            note.rcm,
            DestKind::Transparent,
            [0x11; 32],
            Some(1),
            0,
        );
        let burn = build_egress_burn_from_settle(&opening).map_err(|e| format!("egress: {e:?}"))?;
        apply_egress_burn(&mut self.egress, &burn.public, &burn.witness, Some(&dest))
            .map_err(|e| format!("apply egress: {e:?}"))?;
        self.notes[note_idx].spent = true;

        // Debit escrow, credit user ZEC (lab multi-chain balances)
        let escrow = self
            .accounts
            .accounts
            .iter_mut()
            .find(|a| a.chain == ChainKind::Zcash && a.role == AccountRole::Escrow)
            .ok_or("escrow")?;
        if escrow.balance < note.value {
            return Err("escrow underfunded for egress".into());
        }
        escrow.balance -= note.value;
        let user = self
            .accounts
            .accounts
            .iter_mut()
            .find(|a| a.chain == ChainKind::Zcash && a.role == AccountRole::User)
            .ok_or("user zec")?;
        user.balance = user.balance.saturating_add(note.value);
        Ok(note.value)
    }

    /// Unspent note value for role+asset.
    pub fn note_balance(&self, owner: AccountRole, asset: SeamAsset) -> u128 {
        self.notes
            .iter()
            .filter(|n| n.owner_role == owner && n.asset == asset && !n.spent)
            .map(|n| n.value)
            .sum()
    }
}

/// Route: user BTC mint → swap to ZEC → egress to Zcash user (full corridor cycle).
pub fn route_corridor_btc_to_zec(w: &mut MultiCycleWorld) -> Result<Vec<CycleReceipt>, String> {
    let user_btc = w
        .accounts
        .require(ChainKind::Bitcoin, AccountRole::User)?
        .balance;
    w.mint_note(AccountRole::User, SeamAsset::Btc, user_btc, 1)?;
    let r1 = w.swap_cycle(
        AccountRole::User,
        1,
        SeamAsset::Btc,
        SeamAsset::Zec,
        user_btc / 2,
    )?;
    let zec_out = w.note_balance(AccountRole::User, SeamAsset::Zec);
    assert!(zec_out > 0);
    let burned = w.egress_zec_to_user(AccountRole::User)?;
    assert_eq!(burned, zec_out);
    let zec_user = w
        .accounts
        .require(ChainKind::Zcash, AccountRole::User)?
        .balance;
    assert!(zec_user >= burned);
    Ok(vec![r1])
}

/// Route: tokenfactory alpha minted → swap into BTC SEAM pool → swap to ZEC → multi-cycle back.
pub fn route_tokenfactory_into_corridor(
    w: &mut MultiCycleWorld,
) -> Result<Vec<CycleReceipt>, String> {
    let plan = w
        .tf_plan("pdexalpha")
        .ok_or("pdexalpha plan")?
        .clone();
    // "Mint" TF coins to user then bridge-mint as SEAM
    super::account_curation::credit_tokenfactory_holding(
        &mut w.accounts,
        AccountRole::User,
        &plan,
        5_000_000,
    )?;
    w.mint_note(AccountRole::User, SeamAsset::TfAlpha, 5_000_000, 2)?;
    let mut receipts = Vec::new();
    // TF → BTC
    receipts.push(w.swap_cycle(
        AccountRole::User,
        2,
        SeamAsset::TfAlpha,
        SeamAsset::Btc,
        2_000_000,
    )?);
    // BTC → ZEC
    let btc_bal = w.note_balance(AccountRole::User, SeamAsset::Btc);
    if btc_bal > 0 {
        receipts.push(w.swap_cycle(
            AccountRole::User,
            1,
            SeamAsset::Btc,
            SeamAsset::Zec,
            btc_bal.min(500_000),
        )?);
    }
    Ok(receipts)
}

/// Multi-cycle rebalance: A→B then B→A on same pool (two cycles).
pub fn route_multi_cycle_rebalance(
    w: &mut MultiCycleWorld,
    pool_id: u64,
    a: SeamAsset,
    b: SeamAsset,
    delta: u128,
) -> Result<Vec<CycleReceipt>, String> {
    w.mint_note(AccountRole::User, a, delta * 3, 3)?;
    let r1 = w.swap_cycle(AccountRole::User, pool_id, a, b, delta)?;
    let b_bal = w.note_balance(AccountRole::User, b);
    let r2 = w.swap_cycle(AccountRole::User, pool_id, b, a, b_bal.min(delta))?;
    Ok(vec![r1, r2])
}

/// Oracle cannot mint into multi-cycle world (fail-closed).
pub fn assert_oracle_cannot_mint() {
    let mid = OracleMid {
        pair_key: "BTC-ZEC".into(),
        mid: 1_000_000,
        observed_height: 1,
    };
    assert!(terp_seams::dex::oracle_mint_note(&mid, AssetId::Hub, 1).is_err());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_cycle_corridor_btc_to_zec_egress() {
        let mut w = MultiCycleWorld::seed_lab();
        let receipts = route_corridor_btc_to_zec(&mut w).unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].asset_in, SeamAsset::Btc);
        assert_eq!(receipts[0].asset_out, SeamAsset::Zec);
        assert!(w.accounts.require(ChainKind::Zcash, AccountRole::User).unwrap().balance > 0);
        // User BTC note partially spent / changed
        assert!(w.cycle >= 1);
    }

    #[test]
    fn multi_cycle_tokenfactory_into_corridor() {
        let mut w = MultiCycleWorld::seed_lab();
        let receipts = route_tokenfactory_into_corridor(&mut w).unwrap();
        assert!(!receipts.is_empty());
        assert!(receipts.iter().any(|r| r.asset_in == SeamAsset::TfAlpha));
        // Tokenfactory plan present
        assert!(w.tf_plan("bridgedbtc").is_some());
        assert!(w.tf_plan("pdexalpha").is_some());
    }

    #[test]
    fn multi_cycle_rebalance_two_swaps() {
        let mut w = MultiCycleWorld::seed_lab();
        let r = route_multi_cycle_rebalance(
            &mut w,
            1,
            SeamAsset::Btc,
            SeamAsset::Zec,
            100_000,
        )
        .unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].route.contains("BTC→ZEC"), true);
        assert_eq!(r[1].asset_in, SeamAsset::Zec);
        assert_eq!(r[1].asset_out, SeamAsset::Btc);
        // Two cycles advanced
        assert!(w.cycle >= 2);
    }

    #[test]
    fn multi_cycle_tf_pair_and_oracle_fail_closed() {
        let mut w = MultiCycleWorld::seed_lab();
        w.mint_note(AccountRole::User, SeamAsset::TfAlpha, 1_000_000, 9)
            .unwrap();
        let r = w
            .swap_cycle(
                AccountRole::User,
                3,
                SeamAsset::TfAlpha,
                SeamAsset::TfBeta,
                100_000,
            )
            .unwrap();
        assert!(r.delta_out > 0);
        assert_oracle_cannot_mint();
    }

    #[test]
    fn curated_accounts_seed_all_chains() {
        let w = MultiCycleWorld::seed_lab();
        assert!(w.accounts.get(ChainKind::Bitcoin, AccountRole::Lp).is_some());
        assert!(w.accounts.get(ChainKind::Zcash, AccountRole::Escrow).is_some());
        assert!(w
            .accounts
            .get(ChainKind::Terp, AccountRole::TokenFactoryIssuer)
            .is_some());
        assert_eq!(w.pools.len(), 3);
    }
}
