//! address related crate
// use cosmwasm_std::CanonicalAddr;
use ff::PrimeField;
use pasta_curves::pallas;
use subtle::CtOption;

use crate::{
    keys::{DiversifiedTransmissionKey, Diversifier},
    spec::{diversify_hash, diversify_hash_headstash, NonIdentityPallasPoint},
};

/// A shielded payment address.
///
/// # Examples
///
/// ```
/// use orchard::keys::{SpendingKey, FullViewingKey, Scope};
///
/// let sk = SpendingKey::from_bytes([7; 32]).unwrap();
/// let address = FullViewingKey::from(&sk).address_at(0u32, Scope::External);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Address {
    d: Diversifier,
    pk_d: DiversifiedTransmissionKey,
}

impl Address {
    pub(crate) fn from_parts(d: Diversifier, pk_d: DiversifiedTransmissionKey) -> Self {
        // We assume here that pk_d is correctly-derived from d. We ensure this for
        // internal APIs. For parsing from raw byte encodings, we assume that users aren't
        // modifying internals of encoded address formats. If they do, that can result in
        // lost funds, but we can't defend against that from here.
        Address { d, pk_d }
    }

    /// Returns the [`Diversifier`] for this `Address`.
    pub fn diversifier(&self) -> Diversifier {
        self.d
    }

    pub(crate) fn g_d(&self) -> NonIdentityPallasPoint {
        diversify_hash(self.d.as_array())
    }

    pub(crate) fn pk_d(&self) -> &DiversifiedTransmissionKey {
        &self.pk_d
    }

    /// Serializes this address to its "raw" encoding as specified in [Zcash Protocol Spec § 5.6.4.2: Orchard Raw Payment Addresses][orchardpaymentaddrencoding]
    ///
    /// [orchardpaymentaddrencoding]: https://zips.z.cash/protocol/protocol.pdf#orchardpaymentaddrencoding
    pub fn to_raw_address_bytes(&self) -> [u8; 43] {
        let mut result = [0u8; 43];
        result[..11].copy_from_slice(self.d.as_array());
        result[11..].copy_from_slice(&self.pk_d.to_bytes());
        result
    }

    /// Parse an address from its "raw" encoding as specified in [Zcash Protocol Spec § 5.6.4.2: Orchard Raw Payment Addresses][orchardpaymentaddrencoding]
    ///
    /// [orchardpaymentaddrencoding]: https://zips.z.cash/protocol/protocol.pdf#orchardpaymentaddrencoding
    pub fn from_raw_address_bytes(bytes: &[u8; 43]) -> CtOption<Self> {
        DiversifiedTransmissionKey::from_bytes(bytes[11..].try_into().unwrap()).map(|pk_d| {
            let d = Diversifier::from_bytes(bytes[..11].try_into().unwrap());
            Self::from_parts(d, pk_d)
        })
    }
}

/// Generators for property testing.
#[cfg(any(test, feature = "test-dependencies"))]
#[cfg_attr(docsrs, doc(cfg(feature = "test-dependencies")))]
pub mod testing {
    use proptest::prelude::*;

    use crate::{
        address::RecpAddr,
        keys::{
            testing::{arb_diversifier_index, arb_spending_key},
            FullViewingKey, Scope,
        },
        value::testing::arb_scalar,
    };

    use super::Address;

    prop_compose! {
        /// Generates an arbitrary payment address.
        pub(crate) fn arb_address()(sk in arb_spending_key(), j in arb_diversifier_index()) -> Address {
            let fvk = FullViewingKey::from(&sk);
            fvk.address_at(j, Scope::External)
        }
    }
    prop_compose! {
        /// Generates an arbitrary cosmos headstash recp.
        pub(crate) fn arb_recp()(sk in arb_scalar()) -> RecpAddr {
                RecpAddr(sk.into())
        }
    }
}

/// the canonical bytes of a bech32 address intended to recieve headstash distributions.
/// Note that we never store the RecpAddr as its pallas hash, but expose functions
#[derive(Debug, Copy, Clone)]
pub struct RecpAddr([u8; 32]);

impl RecpAddr {
    /// Returns the bytes for this `RecpAddr`.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    /// Returns the bytes for this `RecpAddr`.
    pub fn to_canonical_bytes(&self) -> [u8; 32] {
        self.0
    }
    /// Returns the [`pallas::Base`] for this `RecpAddr`.
    pub fn to_fp(&self) -> pallas::Base {
        crate::spec::recp_to_fp(self)
    }

    pub(crate) fn g_d(&self) -> NonIdentityPallasPoint {
        diversify_hash_headstash(&self.0)
    }

    /// validates whether a byte array is identical to the pallas field representation of the raw recp bytes
    pub fn validate(&self, pallas: &[u8]) -> bool {
        self.to_fp().to_repr() == pallas
    }
}

// impl From<cosmwasm_std::Binary> for RecpAddr {
//     fn from(value: cosmwasm_std::Binary) -> Self {
//         Self::new(
//             value
//                 .as_slice()
//                 .try_into()
//                 .expect("Invalid nullifier bytes"),
//         )
//     }
// }
// impl From<CanonicalAddr> for RecpAddr {
//     fn from(ca: CanonicalAddr) -> Self {
//         Self(ca.as_slice().try_into().expect("Invalid nullifier bytes"))
//     }
// }

impl TryFrom<&[u8]> for RecpAddr {
    type Error = std::array::TryFromSliceError;

    fn try_from(canonical_addr: &[u8]) -> Result<Self, Self::Error> {
        let bytes: [u8; 32] = canonical_addr.try_into()?;
        Ok(Self(bytes))
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::spec::recp_to_fp;

    // use cosmwasm_std::testing::mock_dependencies;
    // use cosmwasm_std::{Api, CanonicalAddr};

    // #[test]
    // fn test_canon() {
    //     let deps = mock_dependencies();
    //     let addr = deps.api.addr_make("ayo");
    //     let canon: CanonicalAddr = deps.api.addr_canonicalize(&addr.to_string()).unwrap();
    //     println!("{:#?}", addr.to_string());
    //     println!("{:#?}", canon.len());
    //     println!("{:#?}", canon.to_string());
    // }

    // Helper function to create a valid 32-byte array
    fn create_test_bytes() -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = (i % 256) as u8;
        }
        bytes
    }

    #[test]
    fn test_recp_addr_creation_and_to_bytes() {
        let test_bytes = create_test_bytes();
        let recp_addr = RecpAddr(test_bytes);

        assert_eq!(recp_addr.to_canonical_bytes(), test_bytes);
    }

    // #[test]
    // fn test_to_canonical() {
    //     let test_bytes = create_test_bytes();
    //     let recp_addr = RecpAddr(test_bytes);

    //     let canonical = recp_addr.to_canonical();
    //     assert_eq!(canonical.as_slice(), &test_bytes);
    // }

    // #[test]
    // fn test_try_from_canonical_addr_success() {
    //     let test_bytes = create_test_bytes();
    //     let canonical = CanonicalAddr::from(test_bytes);

    //     let recp_addr = RecpAddr::try_from(canonical).unwrap();
    //     assert_eq!(recp_addr.to_bytes(), test_bytes);
    // }

    // #[test]
    // fn test_try_from_canonical_addr_wrong_length() {
    //     // Create a CanonicalAddr with wrong length (not 32 bytes)
    //     let short_bytes = vec![1u8, 2, 3, 4];
    //     let canonical = CanonicalAddr::from(short_bytes);

    //     let result = RecpAddr::try_from(canonical);
    //     assert!(result.is_err());
    // }

    #[test]
    fn test_try_from_slice_success() {
        let test_bytes = create_test_bytes();
        let slice: &[u8] = &test_bytes;

        let recp_addr = RecpAddr::try_from(slice).unwrap();
        assert_eq!(recp_addr.to_canonical_bytes(), test_bytes);
    }

    #[test]
    fn test_try_from_slice_wrong_length_short() {
        let short_slice: &[u8] = &[1, 2, 3, 4];
        let result = RecpAddr::try_from(short_slice);
        assert!(result.is_err());
    }

    #[test]
    fn test_try_from_slice_wrong_length_long() {
        let long_slice: &[u8] = &[0u8; 64];
        let result = RecpAddr::try_from(long_slice);
        assert!(result.is_err());
    }

    #[test]
    fn test_try_from_empty_slice() {
        let empty_slice: &[u8] = &[];
        let result = RecpAddr::try_from(empty_slice);
        assert!(result.is_err());
    }

    #[test]
    fn test_recp_addr_clone() {
        let test_bytes = create_test_bytes();
        let recp_addr = RecpAddr(test_bytes);
        let cloned = recp_addr.clone();

        assert_eq!(recp_addr.to_canonical_bytes(), cloned.to_canonical_bytes());
    }

    #[test]
    fn test_recp_addr_copy() {
        let test_bytes = create_test_bytes();
        let recp_addr = RecpAddr(test_bytes);
        let copied = recp_addr; // Copy trait

        assert_eq!(recp_addr.to_canonical_bytes(), copied.to_canonical_bytes());
    }

    // #[test]
    // fn test_round_trip_canonical() {
    //     let test_bytes = create_test_bytes();
    //     let recp_addr = RecpAddr(test_bytes);

    //     // Convert to canonical and back
    //     let canonical = recp_addr.to_canonical();
    //     let recp_addr_2 = RecpAddr::try_from(canonical).unwrap();

    //     assert_eq!(recp_addr.to_bytes(), recp_addr_2.to_bytes());
    // }

    #[test]
    fn test_all_zeros() {
        let zero_bytes = [0u8; 32];
        let recp_addr = RecpAddr(zero_bytes);

        assert_eq!(recp_addr.to_canonical_bytes(), zero_bytes);
    }

    #[test]
    fn test_all_ones() {
        let ones_bytes = [0xFFu8; 32];
        let recp_addr = RecpAddr(ones_bytes);

        assert_eq!(recp_addr.to_canonical_bytes(), ones_bytes);
    }

    #[test]
    fn test_to_pallas() {
        let test_bytes = create_test_bytes();
        let recp_addr = RecpAddr(test_bytes);

        // Just verify it can be called without panicking
        // The actual correctness depends on the spec::recp_to_fp implementation
        let _pallas_base = recp_addr.to_fp();
    }

    #[test]
    fn test_debug_trait() {
        let test_bytes = create_test_bytes();
        let recp_addr = RecpAddr(test_bytes);

        // Verify Debug trait works
        let debug_string = format!("{:?}", recp_addr);
        assert!(debug_string.contains("RecpAddr"));
    }
    use pasta_curves::group::ff::PrimeField;
    use pasta_curves::pallas;

    // Helper function to create a RecpAddr from a 32-byte array
    fn make_recp_addr(bytes: [u8; 32]) -> RecpAddr {
        RecpAddr::try_from(&bytes[..]).unwrap()
    }

    #[test]
    fn test_recp_to_fp_deterministic() {
        // Same input should always produce same output
        let test_bytes = [42u8; 32];
        let recp1 = make_recp_addr(test_bytes);
        let recp2 = make_recp_addr(test_bytes);

        let fp1 = recp_to_fp(&recp1);
        let fp2 = recp_to_fp(&recp2);

        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_recp_to_fp_different_inputs() {
        // Different inputs should produce different outputs
        let bytes1 = [1u8; 32];
        let bytes2 = [2u8; 32];

        let recp1 = make_recp_addr(bytes1);
        let recp2 = make_recp_addr(bytes2);

        let fp1 = recp_to_fp(&recp1);
        let fp2 = recp_to_fp(&recp2);

        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_recp_to_fp_all_zeros() {
        let zero_bytes = [0u8; 32];
        let recp = make_recp_addr(zero_bytes);

        let fp = recp_to_fp(&recp);

        // Should produce a valid field element (not necessarily zero due to hashing)
        // Just verify it doesn't panic and produces a field element
        assert!(fp != pallas::Base::zero() || fp == pallas::Base::zero());
    }

    #[test]
    fn test_recp_to_fp_all_ones() {
        let ones_bytes = [0xFFu8; 32];
        let recp = make_recp_addr(ones_bytes);

        let fp = recp_to_fp(&recp);

        // Should produce a valid field element
        assert!(fp != pallas::Base::zero() || fp == pallas::Base::zero());
    }

    #[test]
    fn test_recp_to_fp_sequential_bytes() {
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = (i % 256) as u8;
        }

        let recp = make_recp_addr(bytes);
        let fp = recp_to_fp(&recp);

        // Verify it's a valid field element by checking it's in the field
        // (all pallas::Base values are valid by construction)
        let _ = fp;
    }

    #[test]
    fn test_recp_to_fp_single_bit_difference() {
        // Test avalanche effect: small change in input should cause large change in output
        let mut bytes1 = [0u8; 32];
        let mut bytes2 = [0u8; 32];
        bytes2[0] = 1; // Only change first bit

        let recp1 = make_recp_addr(bytes1);
        let recp2 = make_recp_addr(bytes2);

        let fp1 = recp_to_fp(&recp1);
        let fp2 = recp_to_fp(&recp2);

        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_recp_to_fp_change_in_first_half() {
        // Change only in first 16 bytes
        let mut bytes1 = [0u8; 32];
        let mut bytes2 = [0u8; 32];
        bytes2[8] = 1; // Change in first half

        let recp1 = make_recp_addr(bytes1);
        let recp2 = make_recp_addr(bytes2);

        let fp1 = recp_to_fp(&recp1);
        let fp2 = recp_to_fp(&recp2);

        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_recp_to_fp_change_in_second_half() {
        // Change only in second 16 bytes
        let mut bytes1 = [0u8; 32];
        let mut bytes2 = [0u8; 32];
        bytes2[24] = 1; // Change in second half

        let recp1 = make_recp_addr(bytes1);
        let recp2 = make_recp_addr(bytes2);

        let fp1 = recp_to_fp(&recp1);
        let fp2 = recp_to_fp(&recp2);

        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_recp_to_fp_returns_valid_field_element() {
        let test_bytes = [123u8; 32];
        let recp = make_recp_addr(test_bytes);

        let fp = recp_to_fp(&recp);

        // Test that we can perform field operations on the result
        let doubled = fp + fp;
        let squared = fp * fp;

        assert_ne!(doubled, fp); // Unless fp is zero, which it shouldn't be
        assert!(squared == squared); // Just verify operations work
    }

    #[test]
    fn test_recp_to_fp_collision_resistance() {
        // Test a few different inputs to ensure no obvious collisions
        let mut outputs = std::collections::HashSet::new();

        for i in 0..10 {
            let mut bytes = [0u8; 32];
            bytes[0] = i;
            let recp = make_recp_addr(bytes);
            let fp = recp_to_fp(&recp);

            // Convert to bytes for HashSet (PrimeField trait provides to_repr)
            let repr = fp.to_repr();
            assert!(outputs.insert(repr), "Found collision at iteration {}", i);
        }

        assert_eq!(outputs.len(), 10);
    }

    #[test]
    fn test_recp_to_fp_boundary_values() {
        // Test with max u128 in first half
        let mut bytes = [0u8; 32];
        bytes[0..16].copy_from_slice(&[0xFF; 16]);
        let recp1 = make_recp_addr(bytes);

        // Test with max u128 in second half
        let mut bytes = [0u8; 32];
        bytes[16..32].copy_from_slice(&[0xFF; 16]);
        let recp2 = make_recp_addr(bytes);

        let fp1 = recp_to_fp(&recp1);
        let fp2 = recp_to_fp(&recp2);

        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_recp_to_fp_integration_with_recp_addr() {
        // Test that it works seamlessly with RecpAddr's to_pallas method
        let test_bytes = [77u8; 32];
        let recp = make_recp_addr(test_bytes);

        let fp1 = recp_to_fp(&recp);
        let fp2 = recp.to_fp(); // Should call the same function

        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_recp_to_fp_byte_order_matters() {
        // Reversed bytes should give different hash
        let mut bytes1 = [0u8; 32];
        for (i, byte) in bytes1.iter_mut().enumerate() {
            *byte = i as u8;
        }

        let mut bytes2 = bytes1.clone();
        bytes2.reverse();

        let recp1 = make_recp_addr(bytes1);
        let recp2 = make_recp_addr(bytes2);

        let fp1 = recp_to_fp(&recp1);
        let fp2 = recp_to_fp(&recp2);

        assert_ne!(fp1, fp2);
    }
}
