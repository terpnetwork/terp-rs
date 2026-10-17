/// First Principles Bit Decomposition Tests
///
/// This file contains foundational tests to understand:
/// 1. How bit ranges work in Rust
/// 2. How pallas::Base field elements map to bits
/// 3. How to correctly decompose field elements
/// 4. The relationship between L_ORCHARD_BASE (255) and actual bit usage

#[cfg(test)]
mod bit_fundamentals {
    use ff::Field;
    use ff::PrimeFieldBits;
    use pasta_curves::pallas;
    use std::println;
    use std::vec::Vec;

    use crate::constants::L_ORCHARD_BASE;
    #[test]
    fn test_bit_range_syntax() {
        // FUNDAMENTAL: Rust ranges are [start, end) - end is EXCLUSIVE
        let range1 = 0..10; // bits 0,1,2,3,4,5,6,7,8,9 = 10 bits
        let range2 = 250..255; // bits 250,251,252,253,254 = 5 bits
        let range3 = 250..254; // bits 250,251,252,253 = 4 bits

        assert_eq!(range1.len(), 10);
        assert_eq!(range2.len(), 5);
        assert_eq!(range3.len(), 4);

        println!("✓ Range 0..10 contains {} bits", range1.len());
        println!("✓ Range 250..255 contains {} bits", range2.len());
        println!("✓ Range 250..254 contains {} bits", range3.len());
    }

    #[test]
    fn test_field_element_bit_representation() {
        // pallas::Base is a 255-bit field element
        // Valid values are 0 to p-1 where p is the field modulus
        // The modulus is less than 2^255, so not all 255-bit patterns are valid

        // The number 1 in binary
        let one = pallas::Base::one();
        let one_bits: Vec<bool> = one.to_le_bits().iter().by_vals().collect();

        println!("\nField element 1 as bits:");
        println!("  Bit 0 (LSB): {}", one_bits[0]); // Should be true
        println!("  Bit 1: {}", one_bits[1]); // Should be false
        println!("  Bit 2: {}", one_bits[2]); // Should be false
        println!("  Total bits in representation: {}", one_bits.len());

        assert_eq!(one_bits[0], true); // Bit 0 is set
        assert_eq!(one_bits[1], false); // Bit 1 is clear
        assert_eq!(one_bits.len(), 256); // to_le_bits() returns 256 bits (padded)
    }

    #[test]
    fn test_l_orchard_base_255_meaning() {
        // L_ORCHARD_BASE = 255 means we use 255 bits of the field element
        // This is bits 0..255 (inclusive 0-254, exclusive 255)

        // When we take L_ORCHARD_BASE bits, we get bits [0, 255)
        // That's bits 0,1,2,...,253,254 = 255 bits total
        let range = 0..L_ORCHARD_BASE;
        assert_eq!(range.len(), 255);
        println!("\n✓ L_ORCHARD_BASE=255 means bits 0..255");
        println!("  This is bits 0,1,2,...,253,254");
        println!("  Total count: {} bits", range.len());
        // The HIGHEST bit we use is bit 254 (not 255!)
        let highest_bit_index = L_ORCHARD_BASE - 1;
        assert_eq!(highest_bit_index, 254);
        println!("  Highest bit index: {}", highest_bit_index);
    }

    #[test]
    fn test_decomposing_255_bit_field() {
        // If nd is 255 bits (0..255), how do we split it?
        // Option 1: 250 bits + 5 bits
        // Option 2: 250 bits + 4 bits (leaving bit 254 unused)

        // OPTION 1: Use all 255 bits
        let piece_a_range = 0..250; // Bits 0-249 = 250 bits
        let piece_b_range = 250..255; // Bits 250-254 = 5 bits

        assert_eq!(piece_a_range.len(), 250);
        assert_eq!(piece_b_range.len(), 5);
        assert_eq!(piece_a_range.len() + piece_b_range.len(), 255);

        println!("\n✓ Option 1: Split 255 bits as 250+5:");
        println!("  Piece a: bits 0..250 = {} bits", piece_a_range.len());
        println!("  Piece b: bits 250..255 = {} bits", piece_b_range.len());
        println!(
            "  Total: {} bits",
            piece_a_range.len() + piece_b_range.len()
        );

        // OPTION 2: Use only 254 bits (drop highest bit)
        let piece_a_range_alt = 0..250; // Bits 0-249 = 250 bits
        let piece_b_range_alt = 250..254; // Bits 250-253 = 4 bits

        assert_eq!(piece_a_range_alt.len(), 250);
        assert_eq!(piece_b_range_alt.len(), 4);
        assert_eq!(piece_a_range_alt.len() + piece_b_range_alt.len(), 254);

        println!("\n✓ Option 2: Split 254 bits as 250+4:");
        println!("  Piece a: bits 0..250 = {} bits", piece_a_range_alt.len());
        println!(
            "  Piece b: bits 250..254 = {} bits",
            piece_b_range_alt.len()
        );
        println!(
            "  Total: {} bits (bit 254 unused)",
            piece_a_range_alt.len() + piece_b_range_alt.len()
        );
    }

    #[test]
    fn test_actual_field_element_decomposition() {
        // Create a field element with a known bit pattern
        // Let's use 2^250 which has bit 250 set and all others clear

        let two_pow_250 = pallas::Base::from(2u64).pow([250, 0, 0, 0]);
        let bits: Vec<bool> = two_pow_250.to_le_bits().iter().by_vals().collect();

        println!("\n2^250 bit representation:");
        println!("  Bit 249: {}", bits[249]); // Should be false
        println!("  Bit 250: {}", bits[250]); // Should be true
        println!("  Bit 251: {}", bits[251]); // Should be false

        assert_eq!(bits[249], false);
        assert_eq!(bits[250], true);
        assert_eq!(bits[251], false);

        // Extract bits 0..250 (should all be 0)
        let low_250_bits: Vec<bool> = bits.iter().take(250).copied().collect();
        assert!(low_250_bits.iter().all(|&b| !b));
        println!("  ✓ Bits 0..250 are all zero");

        // Extract bits 250..255 (bit 250 should be set)
        let high_5_bits: Vec<bool> = bits.iter().skip(250).take(5).copied().collect();
        assert_eq!(high_5_bits[0], true); // Bit 250
        assert_eq!(high_5_bits[1], false); // Bit 251
        println!("  ✓ Bits 250..255: first bit is set");
    }

    #[test]
    fn test_reconstructing_from_pieces() {
        // If we split a value into pieces, can we reconstruct it?

        use ff::Field;

        // Original value: let's use a simple example
        let original = pallas::Base::from(12345u64);
        let original_bits: Vec<bool> = original.to_le_bits().iter().by_vals().take(255).collect();

        // Split into piece_a (bits 0..250) and piece_b (bits 250..255)
        let piece_a_bits: Vec<bool> = original_bits.iter().take(250).copied().collect();
        let piece_b_bits: Vec<bool> = original_bits.iter().skip(250).take(5).copied().collect();

        // Convert pieces back to field elements
        let piece_a_value = bits_to_field_element(&piece_a_bits);
        let piece_b_value = bits_to_field_element(&piece_b_bits);

        // Reconstruct: original = piece_a + piece_b * 2^250
        let two_pow_250 = pallas::Base::from(2u64).pow([250, 0, 0, 0]);
        let reconstructed = piece_a_value + piece_b_value * two_pow_250;

        println!("\nReconstruction test:");
        println!("  Original: {:?}", original);
        println!("  Piece A (bits 0..250): {:?}", piece_a_value);
        println!("  Piece B (bits 250..255): {:?}", piece_b_value);
        println!("  Reconstructed: {:?}", reconstructed);

        assert_eq!(original, reconstructed);
        println!("  ✓ Successfully reconstructed!");
    }

    #[test]
    fn test_sinsemilla_piece_requirements() {
        // Sinsemilla pieces must be multiples of 10 bits for running sum constraints

        let valid_piece_sizes = vec![10, 20, 60, 130, 250];
        let invalid_piece_sizes = vec![5, 15, 55, 255];

        println!("\nSinsemilla piece size requirements:");
        println!("  Valid sizes (multiples of 10):");
        for size in valid_piece_sizes {
            assert_eq!(size % 10, 0);
            println!("    {} bits ✓", size);
        }

        println!("  Invalid sizes (NOT multiples of 10):");
        for size in invalid_piece_sizes {
            assert_ne!(size % 10, 0);
            println!("    {} bits ✗", size);
        }

        // For decomposing a 255-bit field:
        // Option 1: 250 + 10 = 260 bits (need 5 bits padding)
        // Option 2: 250 + 60 = 310 bits (need more data)
        println!("\n  For 255-bit nd field:");
        println!("    250-bit piece a ✓");
        println!("    5-bit remainder ✗ (need to pad to 10)");
        println!("    Solution: 250 + (5 nd bits + 5 padding) = 250 + 10 ✓");
    }

    // Helper function to convert bits to field element
    fn bits_to_field_element(bits: &[bool]) -> pallas::Base {
        let mut result = pallas::Base::zero();
        let mut power = pallas::Base::one();
        let two = pallas::Base::from(2u64);

        for &bit in bits {
            if bit {
                result += power;
            }
            power *= two;
        }
        result
    }

    #[test]
    fn test_correct_nd_decomposition_for_headstash() {
        // Based on L_ORCHARD_BASE = 255, nd has 255 usable bits (0..255)
        // We want to decompose this for Sinsemilla (multiples of 10)

        const L_ORCHARD_BASE: usize = 255;

        println!("\n=== CORRECT nd DECOMPOSITION ===");
        println!(
            "nd is a {}-bit field element (bits 0..{})",
            L_ORCHARD_BASE, L_ORCHARD_BASE
        );
        println!(
            "This means bits 0,1,2,...,253,254 ({} bits total)",
            L_ORCHARD_BASE
        );

        // Decomposition:
        // Piece a: bits 0..250 = 250 bits ✓ (multiple of 10)
        // Remaining: bits 250..255 = 5 bits ✗ (NOT multiple of 10)
        // Solution: Piece b: bits 250..255 + 5 zero bits = 10 bits ✓

        let piece_a = 0..250;
        let piece_b_nd_portion = 250..255;
        let piece_b_padding = 5; // zero bits

        assert_eq!(piece_a.len(), 250);
        assert_eq!(piece_b_nd_portion.len(), 5);
        assert_eq!((piece_b_nd_portion.len() + piece_b_padding) % 10, 0);

        println!("\n✓ CORRECT decomposition:");
        println!("  Piece a: nd[0..250]   = {} bits", piece_a.len());
        println!(
            "  Piece b: nd[250..255] = {} bits",
            piece_b_nd_portion.len()
        );
        println!("           + {} zero bits padding", piece_b_padding);
        println!(
            "           = {} bits total ✓",
            piece_b_nd_portion.len() + piece_b_padding
        );
        println!(
            "\nTotal nd bits used: {} out of {}",
            piece_a.len() + piece_b_nd_portion.len(),
            L_ORCHARD_BASE
        );
    }

    #[test]
    fn test_wrong_decompositions_to_avoid() {
        println!("\n=== WRONG DECOMPOSITIONS (DON'T DO THIS) ===");

        // WRONG 1: Using 254 bits (0..254) and leaving bit 254 unused
        let _wrong1_a = 0..250;
        let wrong1_b = 250..254; // Only 4 bits!
        println!("\n✗ WRONG: 250 + 4 bits");
        println!("  Problem: 4 is not a multiple of 10");
        println!("  Missing: bit 254 of nd is unused");
        assert_eq!(wrong1_b.len(), 4);
        assert_ne!(wrong1_b.len() % 10, 0);

        // WRONG 2: Trying to use 256 bits (0..256)
        // This would try to use bit 255 which doesn't exist in a 255-bit field!
        println!("\n✗ WRONG: trying to use bits 0..256");
        println!("  Problem: nd only has 255 bits (0..255)");
        println!("  Bit 255 doesn't exist!");

        // WRONG 3: Using piece sizes that aren't multiples of 10
        let wrong3_a = 0..245;
        let _wrong3_b = 245..255;
        println!("\n✗ WRONG: 245 + 10 bits");
        println!("  Problem: 245 is not a multiple of 10");
        assert_ne!(wrong3_a.len() % 10, 0);
    }
}
