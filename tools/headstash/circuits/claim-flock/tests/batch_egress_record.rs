//! Local persist of one batch egress. Nothing here is broadcast.

use std::fs;
use std::path::Path;

use headstash_claim::batch::{Egress, Payout, egress};

/// Chain-facing epoch record: nullifier root, rejected count, payout list.
struct PersistedEgress {
    nullifier_root: [u32; 8],
    rejected: u32,
    payouts: Vec<([u32; 5], u32)>,
}

impl PersistedEgress {
    fn from_egress(egress: &Egress) -> Self {
        Self {
            nullifier_root: egress.nullifier_root,
            rejected: egress.rejected,
            payouts: egress.payouts.clone(),
        }
    }

    fn to_json(&self) -> String {
        let mut payouts = String::new();
        for (i, (destination, amount)) in self.payouts.iter().enumerate() {
            if i > 0 {
                payouts.push(',');
            }
            payouts.push_str(&format!(
                "{{\"destination\":{},\"amount\":{amount}}}",
                json_u32s(destination),
            ));
        }
        format!(
            "{{\"nullifier_root\":{},\"rejected\":{},\"payouts\":[{payouts}]}}",
            json_u32s(&self.nullifier_root),
            self.rejected,
        )
    }

    fn write(&self, path: &Path) -> String {
        let body = self.to_json();
        fs::write(path, &body).expect("persist egress record");
        body
    }
}

fn json_u32s(words: &[u32]) -> String {
    let parts: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    format!("[{}]", parts.join(","))
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let rest = text
        .split_once(start)
        .unwrap_or_else(|| panic!("missing {start}"))
        .1;
    rest.split_once(end)
        .unwrap_or_else(|| panic!("missing {end} after {start}"))
        .0
}

fn parse_u32_list(text: &str) -> Vec<u32> {
    text.split(',')
        .map(|part| part.trim().parse().expect("u32"))
        .collect()
}

fn accepted_payout() -> Payout {
    Payout {
        nullifier: [9, 8, 7, 6, 5, 4, 3, 2],
        destination: [
            0x0a0a_0a0a,
            0x0b0b_0b0b,
            0x0c0c_0c0c,
            0x0d0d_0d0d,
            0x0e0e_0e0e,
        ],
        amount: 42,
    }
}

/// One accepted payout and one rejection become a local record of the
/// nullifier root and the `(destination, amount)` list. Proof bytes and
/// `rseed` are not in that record. This test does not broadcast.
#[test]
fn persisted_egress_one_accepted_one_rejected() {
    let rseed: [u32; 8] = [
        0xA11C_E001,
        0xA11C_E002,
        0xA11C_E003,
        0xA11C_E004,
        0xA11C_E005,
        0xA11C_E006,
        0xA11C_E007,
        0xA11C_E008,
    ];
    let proof_bytes = b"LIGERITO-PROOF-BYTES";

    let accepted = [accepted_payout()];
    let folded = egress(&accepted, 1);
    let uncounted = egress(&accepted, 0);
    assert_eq!(folded.rejected, 1);
    assert_eq!(folded.payouts.len(), 1);
    assert_eq!(
        folded.payouts[0],
        (accepted[0].destination, accepted[0].amount)
    );
    assert_ne!(folded.nullifier_root, [0u32; 8]);
    // A rejection is counted. It is not hashed into the root.
    assert_eq!(folded.nullifier_root, uncounted.nullifier_root);

    let record = PersistedEgress::from_egress(&folded);
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("batch-egress-record.json");
    let written = record.write(&path);
    let loaded = fs::read_to_string(&path).expect("read persisted egress");
    assert_eq!(loaded, written);

    let root = parse_u32_list(between(&loaded, "\"nullifier_root\":[", "]"));
    assert_eq!(root, folded.nullifier_root.to_vec());
    let rejected: u32 = between(&loaded, "\"rejected\":", ",").parse().unwrap();
    assert_eq!(rejected, 1);
    let destination = parse_u32_list(between(&loaded, "\"destination\":[", "]"));
    assert_eq!(destination, accepted[0].destination.to_vec());
    let amount: u32 = between(&loaded, "\"amount\":", "}").parse().unwrap();
    assert_eq!(amount, accepted[0].amount);
    assert_eq!(loaded.matches("\"destination\"").count(), 1);

    let file = fs::read(&path).expect("record bytes");
    assert!(
        !file
            .windows(proof_bytes.len())
            .any(|window| window == proof_bytes),
        "proof bytes must not be persisted"
    );
    assert!(!loaded.to_ascii_lowercase().contains("rseed"));
    assert!(!loaded.to_ascii_lowercase().contains("proof"));
    assert!(!loaded.contains(&json_u32s(&rseed)));
    assert!(!loaded.contains(&json_u32s(&accepted[0].nullifier)));
}
