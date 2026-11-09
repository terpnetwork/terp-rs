//! Print the paymint whitelist tree for a payment minter. Offline; no chain access.
//! cargo run -p cw-infuser-scripts --bin paymint_tree -- <payment_minter> <alloc> <salt>
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        a.len() == 3,
        "usage: paymint_tree <payment_minter> <alloc> <salt>"
    );
    let t = cw_infuser_scripts::paymint::paymint_tree(&a[0], a[1].parse()?, &a[2]);
    println!(
        "{}",
        serde_json::json!({"member": format!("{}{}", a[0], t.alloc), "merkle_root": t.root, "proof_hashes": t.proof, "alloc": t.alloc})
    );
    Ok(())
}
