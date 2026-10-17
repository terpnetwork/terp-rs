//! Multi-chain **account curation + seeding** for private-DEX / bridge harness elevation.
//!
//! Curates lab identities and balances across **Bitcoin**, **Zcash**, and **Terp** so
//! multi-cycle seam routes (mint → swap → egress → reverse) can be exercised with
//! explicit, inspectable accounts — not ad-hoc strings per test.
//!
//! Tokenfactory plans describe how Terp denoms map to 32-byte private-DEX asset ids
//! (same mapping as `deploy_private_dex_testnet`: `sha256(denom)`).
//!
//! SSOT: `docs/private-bridge/DESIGN-HARNESS-ACCOUNT-CURATION-MULTI-CYCLE-2026-07-22.md`

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Home / claim chain for a curated account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChainKind {
    Bitcoin,
    Zcash,
    Terp,
}

impl ChainKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChainKind::Bitcoin => "bitcoin",
            ChainKind::Zcash => "zcash",
            ChainKind::Terp => "terp",
        }
    }
}

/// Role in bridge / private-DEX / tokenfactory demos.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccountRole {
    /// End-user corridor identity (Cash App–style depositor / swapper).
    User,
    /// Liquidity provider (bridged LP notes / pool seed).
    Lp,
    /// Protocol escrow (ZEC FROST wallet / BTC lock bucket).
    Escrow,
    /// Relayer / harness broadcaster (no product custody claim).
    Relayer,
    /// Private-DEX admin / pool creator on Terp.
    DexAdmin,
    /// Tokenfactory issuer (create_denom + mint on Terp).
    TokenFactoryIssuer,
}

impl AccountRole {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountRole::User => "user",
            AccountRole::Lp => "lp",
            AccountRole::Escrow => "escrow",
            AccountRole::Relayer => "relayer",
            AccountRole::DexAdmin => "dex_admin",
            AccountRole::TokenFactoryIssuer => "tokenfactory_issuer",
        }
    }
}

/// Seeded lab account (deterministic labels for pure tests; multi-net may fill live addresses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeededAccount {
    pub chain: ChainKind,
    pub role: AccountRole,
    /// Display / RPC address or lab label.
    pub address: String,
    /// Optional 32B owner binding (Terp notes / dest seal).
    pub owner_binding: Option<[u8; 32]>,
    /// Liquid balance in chain base units (sats / zat / uterp-like).
    pub balance: u128,
    /// Asset tag for multi-asset accounts (`BTC`, `ZEC`, `factory/...`, `uterp`).
    pub asset_tag: String,
}

/// Tokenfactory denom plan (Terp) for seam asset_ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenFactoryDenomPlan {
    pub creator: String,
    pub subdenom: String,
    /// Full denom `factory/{creator}/{subdenom}`.
    pub denom: String,
    /// `sha256(denom)` — private-DEX asset id (same as deploy_private_dex_testnet).
    pub asset_id: [u8; 32],
    pub mint_amount: u128,
}

impl TokenFactoryDenomPlan {
    pub fn new(creator: impl Into<String>, subdenom: impl Into<String>, mint_amount: u128) -> Self {
        let creator = creator.into();
        let subdenom = subdenom.into();
        let denom = format!("factory/{creator}/{subdenom}");
        let asset_id = asset_id_from_denom(&denom);
        Self {
            creator,
            subdenom,
            denom,
            asset_id,
            mint_amount,
        }
    }
}

/// Map denom → 32-byte asset id (SSOT with deploy_private_dex_testnet).
pub fn asset_id_from_denom(denom: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(denom.as_bytes()));
    out
}

/// Deterministic lab binding from (chain, role, salt).
pub fn lab_owner_binding(chain: ChainKind, role: AccountRole, salt: u8) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"terp-harness-account-v0");
    h.update(chain.as_str().as_bytes());
    h.update(role.as_str().as_bytes());
    h.update([salt]);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

fn lab_address(chain: ChainKind, role: AccountRole, idx: u8) -> String {
    format!("lab:{}:{}:{}", chain.as_str(), role.as_str(), idx)
}

/// Full multi-chain registry after curation.
#[derive(Clone, Debug, Default)]
pub struct AccountRegistry {
    pub accounts: Vec<SeededAccount>,
    pub tokenfactory: Vec<TokenFactoryDenomPlan>,
}

impl AccountRegistry {
    pub fn get(&self, chain: ChainKind, role: AccountRole) -> Option<&SeededAccount> {
        self.accounts
            .iter()
            .find(|a| a.chain == chain && a.role == role)
    }

    pub fn require(&self, chain: ChainKind, role: AccountRole) -> Result<&SeededAccount, String> {
        self.get(chain, role).ok_or_else(|| {
            format!("missing curated account {}/{}", chain.as_str(), role.as_str())
        })
    }

    pub fn tokenfactory_by_subdenom(&self, sub: &str) -> Option<&TokenFactoryDenomPlan> {
        self.tokenfactory.iter().find(|t| t.subdenom == sub)
    }

    /// Snapshot balances by (chain, role) for multi-cycle assertions.
    pub fn balance_map(&self) -> BTreeMap<(ChainKind, AccountRole), u128> {
        let mut m = BTreeMap::new();
        for a in &self.accounts {
            *m.entry((a.chain, a.role)).or_default() += a.balance;
        }
        m
    }
}

/// Lab fixture amounts (aligned with omni LP / user fixtures where useful).
pub const LAB_BTC_LP_SATS: u128 = 10_000_000;
pub const LAB_BTC_USER_SATS: u128 = 1_000_000;
pub const LAB_ZEC_ESCROW_ZATS: u128 = 55_000_000;
pub const LAB_ZEC_USER_ZATS: u128 = 0; // user receives after egress
pub const LAB_TERP_ADMIN_UTERP: u128 = 1_000_000_000_000;
pub const LAB_TF_MINT: u128 = 1_000_000_000_000; // 1e12 like deploy_private_dex_testnet

/// Curate a full lab multi-chain account set + tokenfactory asset plans.
///
/// Pure: no Docker. Multi-net hooks may later overwrite `address` fields with live values.
pub fn curate_lab_accounts() -> AccountRegistry {
    let issuer = lab_address(ChainKind::Terp, AccountRole::TokenFactoryIssuer, 0);
    let mut reg = AccountRegistry::default();

    // Bitcoin
    reg.accounts.push(SeededAccount {
        chain: ChainKind::Bitcoin,
        role: AccountRole::Lp,
        address: lab_address(ChainKind::Bitcoin, AccountRole::Lp, 0),
        owner_binding: Some(lab_owner_binding(ChainKind::Bitcoin, AccountRole::Lp, 0)),
        balance: LAB_BTC_LP_SATS,
        asset_tag: "BTC".into(),
    });
    reg.accounts.push(SeededAccount {
        chain: ChainKind::Bitcoin,
        role: AccountRole::User,
        address: lab_address(ChainKind::Bitcoin, AccountRole::User, 0),
        owner_binding: Some(lab_owner_binding(ChainKind::Bitcoin, AccountRole::User, 0)),
        balance: LAB_BTC_USER_SATS,
        asset_tag: "BTC".into(),
    });

    // Zcash (FROST escrow + user dest)
    reg.accounts.push(SeededAccount {
        chain: ChainKind::Zcash,
        role: AccountRole::Escrow,
        address: lab_address(ChainKind::Zcash, AccountRole::Escrow, 0),
        owner_binding: Some(lab_owner_binding(ChainKind::Zcash, AccountRole::Escrow, 0)),
        balance: LAB_ZEC_ESCROW_ZATS,
        asset_tag: "ZEC".into(),
    });
    reg.accounts.push(SeededAccount {
        chain: ChainKind::Zcash,
        role: AccountRole::User,
        address: lab_address(ChainKind::Zcash, AccountRole::User, 0),
        owner_binding: Some(lab_owner_binding(ChainKind::Zcash, AccountRole::User, 0)),
        balance: LAB_ZEC_USER_ZATS,
        asset_tag: "ZEC".into(),
    });

    // Terp
    reg.accounts.push(SeededAccount {
        chain: ChainKind::Terp,
        role: AccountRole::DexAdmin,
        address: lab_address(ChainKind::Terp, AccountRole::DexAdmin, 0),
        owner_binding: Some(lab_owner_binding(ChainKind::Terp, AccountRole::DexAdmin, 0)),
        balance: LAB_TERP_ADMIN_UTERP,
        asset_tag: "uterp".into(),
    });
    reg.accounts.push(SeededAccount {
        chain: ChainKind::Terp,
        role: AccountRole::TokenFactoryIssuer,
        address: issuer.clone(),
        owner_binding: Some(lab_owner_binding(
            ChainKind::Terp,
            AccountRole::TokenFactoryIssuer,
            0,
        )),
        balance: LAB_TERP_ADMIN_UTERP,
        asset_tag: "uterp".into(),
    });
    reg.accounts.push(SeededAccount {
        chain: ChainKind::Terp,
        role: AccountRole::User,
        address: lab_address(ChainKind::Terp, AccountRole::User, 0),
        owner_binding: Some(lab_owner_binding(ChainKind::Terp, AccountRole::User, 0)),
        balance: 0,
        asset_tag: "uterp".into(),
    });
    reg.accounts.push(SeededAccount {
        chain: ChainKind::Terp,
        role: AccountRole::Relayer,
        address: lab_address(ChainKind::Terp, AccountRole::Relayer, 0),
        owner_binding: None,
        balance: 0,
        asset_tag: "uterp".into(),
    });

    // Tokenfactory demo assets (mirror deploy_private_dex_testnet subdenoms)
    for sub in ["pdexalpha", "pdexbeta", "pdexgamma", "bridgedbtc", "bridgedzec"] {
        reg.tokenfactory
            .push(TokenFactoryDenomPlan::new(&issuer, sub, LAB_TF_MINT));
    }

    reg
}

/// Apply a tokenfactory "mint" into Terp user balance tracking for a denom (pure ledger).
pub fn credit_tokenfactory_holding(
    reg: &mut AccountRegistry,
    role: AccountRole,
    plan: &TokenFactoryDenomPlan,
    amount: u128,
) -> Result<(), String> {
    let idx = reg
        .accounts
        .iter()
        .position(|a| a.chain == ChainKind::Terp && a.role == role)
        .ok_or_else(|| format!("no terp account for {}", role.as_str()))?;
    let can_merge =
        reg.accounts[idx].asset_tag == plan.denom || reg.accounts[idx].balance == 0;
    if can_merge {
        reg.accounts[idx].asset_tag = plan.denom.clone();
        reg.accounts[idx].balance = reg.accounts[idx].balance.saturating_add(amount);
    } else {
        let base = &reg.accounts[idx];
        reg.accounts.push(SeededAccount {
            chain: ChainKind::Terp,
            role,
            address: format!("{}:{}", base.address, plan.subdenom),
            owner_binding: base.owner_binding,
            balance: amount,
            asset_tag: plan.denom.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curate_lab_has_btc_zec_terp_roles() {
        let reg = curate_lab_accounts();
        assert!(reg.get(ChainKind::Bitcoin, AccountRole::User).is_some());
        assert!(reg.get(ChainKind::Bitcoin, AccountRole::Lp).is_some());
        assert!(reg.get(ChainKind::Zcash, AccountRole::Escrow).is_some());
        assert!(reg.get(ChainKind::Terp, AccountRole::DexAdmin).is_some());
        assert!(reg
            .get(ChainKind::Terp, AccountRole::TokenFactoryIssuer)
            .is_some());
        assert!(reg.tokenfactory.len() >= 5);
        let alpha = reg.tokenfactory_by_subdenom("pdexalpha").unwrap();
        assert!(alpha.denom.starts_with("factory/"));
        assert_eq!(alpha.asset_id, asset_id_from_denom(&alpha.denom));
    }

    #[test]
    fn tokenfactory_asset_id_matches_sha256_denom() {
        let p = TokenFactoryDenomPlan::new("terp1issuer", "foo", 100);
        let mut expect = [0u8; 32];
        expect.copy_from_slice(&Sha256::digest(p.denom.as_bytes()));
        assert_eq!(p.asset_id, expect);
    }

    #[test]
    fn credit_tokenfactory_holding_updates_user() {
        let mut reg = curate_lab_accounts();
        let plan = reg.tokenfactory_by_subdenom("bridgedbtc").unwrap().clone();
        credit_tokenfactory_holding(&mut reg, AccountRole::User, &plan, 1_000).unwrap();
        let u = reg
            .accounts
            .iter()
            .find(|a| a.chain == ChainKind::Terp && a.role == AccountRole::User && a.balance > 0)
            .unwrap();
        assert!(u.balance >= 1_000);
    }
}
