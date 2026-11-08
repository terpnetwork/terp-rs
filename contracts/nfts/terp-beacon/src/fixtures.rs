pub use crate::networks::DrandNetwork as ChainFixture;

pub struct RoundFixture {
    /// URL the round JSON was fetched from.
    pub source_url: &'static str,
    pub round: u64,
    pub randomness_hex: &'static str,
    pub signature_hex: &'static str,
    pub previous_signature_hex: Option<&'static str>,
    /// `genesis + (round - 1) * period`, in unix seconds.
    pub published_unix: u64,
}

/// League of Entropy mainnet "default". Same value as [`crate::networks::DEFAULT`].
pub const DEFAULT_CHAIN: ChainFixture = crate::networks::DEFAULT;

/// Quicknet. Same value as [`crate::networks::QUICKNET`].
pub const QUICKNET_CHAIN: ChainFixture = crate::networks::QUICKNET;

/// https://api.drand.sh/8990e7a9aaed2ffed73dbd7092123d6f289930540d7651336225dc172e51b2ce/public/6611295
pub const DEFAULT_ROUND: RoundFixture = RoundFixture {
    source_url: "https://api.drand.sh/8990e7a9aaed2ffed73dbd7092123d6f289930540d7651336225dc172e51b2ce/public/6611295",
    round: 6_611_295,
    randomness_hex: "54c308d4cabeae6f11a9077c3051181a51028de394c205df4cc851b82c7d0116",
    signature_hex: "b09f499631ee78b28d65fa2fbeb1face1574c7167d2c696a9969b294867e973db82d5a3c19f4279fed884a202050edf6038bb3c1fbbd3d5a92ae3eb9f55f4c4eea8af329ba6c3804cd481b865d7fe3da242e5176db4a23700a02c8a28049a297",
    previous_signature_hex: Some(
        "884fdec24063045af3845f86fd39a5770fd8e6d70649bdfa746dcbef5065521f82e236d4c355ea2baf3e2febacecde22149f9d54b1f71b8190638437ea4987ef574125ed9f2ad28d70a6de5c07d1827b71a6f4772b9206ab2c2558f5d5a8ed37",
    ),
    published_unix: 1_793_769_870,
};

/// https://api.drand.sh/52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971/public/33655511
pub const QUICKNET_ROUND: RoundFixture = RoundFixture {
    source_url: "https://api.drand.sh/52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971/public/33655511",
    round: 33_655_511,
    randomness_hex: "e8909f9607b19b365f4dfcc02683151e8c752f482c93299b103263836a3f6755",
    signature_hex: "8cbec8c08a98c04fd0d6215df7ace810e7bca69d30e92eb2fe3b14a3b29b061d599e5cc98ed894d5a4628646cbdbbaaa",
    previous_signature_hex: None,
    published_unix: 1_793_769_897,
};

/// BLS12-381 G1 generator, compressed. A valid point that is not the default chain key.
pub const UNRELATED_G1_PUBKEY_HEX: &str = "97f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb";

/// Fastnet (bls-unchained-on-g1) group key. A valid G2 point that is not the quicknet key.
/// https://api.drand.sh/dbd506d6ef76e5f386f41c651dcb808c5bcbd75471cc4eafa3f4df7ad4e4c493/info
pub const UNRELATED_G2_PUBKEY_HEX: &str = "a0b862a7527fee3a731bcb59280ab6abd62d5c0b6ea03dc4ddf6612fdfc9d01f01c31542541771903475eb1ec6615f8d0df0b8b6dce385811d6dcf8cbefb8759e5e616a3dfd054c928940766d9a5b9db91e3b697e5d70a975181e007f87fca5e";
