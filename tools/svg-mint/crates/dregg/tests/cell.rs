use dregg::cell::{Cell, CellError, Phase, Turn};
use dregg::checkout;
use dregg::commit::{
    bind, commit_action, commit_deposit, nullifier, ActionOpening, DepositOpening, ForeignWitness,
    Rail,
};
use dregg::mentor::{Mentor, MentorOpening};

fn deposit(rail: Rail, amount: u128, raw: &[u8]) -> DepositOpening {
    DepositOpening {
        witness: ForeignWitness {
            rail,
            amount,
            raw: raw.to_vec(),
            salt: [7u8; 32],
        },
    }
}

fn deposit_salt(rail: Rail, amount: u128, raw: &[u8], salt: u8) -> DepositOpening {
    DepositOpening {
        witness: ForeignWitness {
            rail,
            amount,
            raw: raw.to_vec(),
            salt: [salt; 32],
        },
    }
}

fn action(session: [u8; 32]) -> ActionOpening {
    ActionOpening {
        collection: "svg".into(),
        svg_hash: [9u8; 32],
        recipient: "terp1mintrecipientxxxxxxxxxxxxxxxxxxxxxx".into(),
        session,
        salt: [3u8; 32],
    }
}

fn ready_cell(max_supply: u64) -> (std::sync::MutexGuard<'static, ()>, Cell) {
    let g = checkout::minter_env_lock();
    std::env::remove_var("DREGG_MINTER_LCD");
    std::env::remove_var("DREGG_MINTER_ADDR");
    std::env::remove_var("DREGG_MINTER_FUNDED");
    std::env::set_var(
        "DREGG_MINTER_BALANCE",
        checkout::native_needed_for_nfts(1_000).to_string(),
    );
    let mut cell = Cell::new();
    cell.apply(Turn::SetCollection {
        id: "svg".into(),
        max_supply,
    })
    .unwrap();
    (g, cell)
}

#[test]
fn triple_pay_qty_and_stale_refund() {
    let (_g, mut cell) = ready_cell(100);
    // 3× BTC unit (1500 sats)
    let d = deposit(Rail::Btc, 4_500, b"btc-triple");
    let n = nullifier(&d);
    cell.apply(Turn::DepositSeen { opening: d }).unwrap();
    assert_eq!(cell.qty(&n), Some(3));
    // tokens = floor(paid/unit); BOGO via nfts_for_units; mint_batches splits qty
    assert_eq!(checkout::units_paid(4_500, 1_500), 3);
    let nfts = checkout::nfts_for_units(3, 0);
    assert_eq!(nfts, 6);
    assert_eq!(checkout::mint_batches(nfts, 5), vec![5, 1]);
    assert_eq!(checkout::mint_batches(12, 5), vec![5, 5, 2]);
    cell.apply(Turn::Tick { now: 1_000 }).unwrap();
    cell.apply(Turn::MintEntitled {
        n_deposit: n,
        action: action([2u8; 32]),
    })
    .unwrap();
    cell.apply(Turn::Tick { now: 1_000 + 20 * 60 }).unwrap();
    cell.apply(Turn::SweepStaleEntitlements {
        timeout_secs: 20 * 60,
    })
    .unwrap();
    assert_eq!(cell.phase(&n), Some(Phase::Refunded));
}

#[test]
fn sweep_before_timeout_keeps_entitled() {
    let (_g, mut cell) = ready_cell(100);
    let d = deposit(Rail::Btc, 1_500, b"btc-fresh");
    let n = nullifier(&d);
    cell.apply(Turn::DepositSeen { opening: d }).unwrap();
    cell.apply(Turn::Tick { now: 100 }).unwrap();
    cell.apply(Turn::MintEntitled {
        n_deposit: n,
        action: action([2u8; 32]),
    })
    .unwrap();
    cell.apply(Turn::Tick { now: 100 + 60 }).unwrap();
    cell.apply(Turn::SweepStaleEntitlements {
        timeout_secs: 20 * 60,
    })
    .unwrap();
    assert_eq!(cell.phase(&n), Some(Phase::Entitled));
}

fn happy_path_fixture() {
    let (_g, mut cell) = ready_cell(100);
    let d = deposit(Rail::ZecIronwood, 150_000, b"note-cm-lab");
    let n = nullifier(&d);
    cell.apply(Turn::DepositSeen { opening: d.clone() })
        .unwrap();
    let sess = [1u8; 32];
    let a = action(sess);
    cell.apply(Turn::MintEntitled {
        n_deposit: n,
        action: a.clone(),
    })
    .unwrap();
    let packet = cell
        .apply(Turn::MintSettled {
            n_deposit: n,
            token_id: "1".into(),
            terp_tx: "lab".into(),
            receipt: *blake3::hash(b"settle-ok").as_bytes(),
        })
        .unwrap()
        .expect("packet");
    let json = serde_json::to_string(&packet).unwrap();
    assert!(!packet.leaks_payment_source(&json));
    assert!(!json.contains("note-cm"));
    assert!(!json.contains("txid"));
    assert_eq!(packet.c_deposit, commit_deposit(&d));
    assert_eq!(packet.c_action, commit_action(&a));
    assert_eq!(packet.bind, bind(&packet.c_deposit, &packet.c_action, &sess));
}

#[test]
fn double_spend_refused() {
    let (_g, mut cell) = ready_cell(100);
    let d = deposit(Rail::Eth, 1_000_000_000_000_000, b"log");
    cell.apply(Turn::DepositSeen { opening: d.clone() }).unwrap();
    let err = cell
        .apply(Turn::DepositSeen { opening: d })
        .unwrap_err();
    assert_eq!(err, CellError::DoubleSpend);
}

#[test]
fn entitle_without_deposit_refused() {
    let (_g, mut cell) = ready_cell(100);
    let err = cell
        .apply(Turn::MintEntitled {
            n_deposit: [0u8; 32],
            action: action([1u8; 32]),
        })
        .unwrap_err();
    assert_eq!(err, CellError::NoDeposit);
}

#[test]
fn underpay_ignored() {
    let (_g, mut cell) = ready_cell(100);
    let d = deposit(Rail::Btc, 1, b"outpoint");
    let n = nullifier(&d);
    assert_eq!(
        cell.apply(Turn::DepositSeen { opening: d }).unwrap_err(),
        CellError::Underpay
    );
    assert!(cell.phase(&n).is_none());
}

#[test]
fn deposit_refused_when_minter_unfunded() {
    let _g = checkout::minter_env_lock();
    std::env::remove_var("DREGG_MINTER_LCD");
    std::env::remove_var("DREGG_MINTER_ADDR");
    std::env::remove_var("DREGG_MINTER_FUNDED");
    std::env::remove_var("DREGG_MINTER_BALANCE");
    let mut cell = Cell::new();
    cell.apply(Turn::SetCollection {
        id: "svg".into(),
        max_supply: 100,
    })
    .unwrap();
    let d = deposit(Rail::Btc, 1_500, b"btc-closed");
    let n = nullifier(&d);
    assert_eq!(
        cell.apply(Turn::DepositSeen { opening: d }).unwrap_err(),
        CellError::CheckoutClosed
    );
    assert!(cell.phase(&n).is_none());
}

#[test]
fn entitle_refused_when_checkout_unbound() {
    let _g = checkout::minter_env_lock();
    std::env::remove_var("DREGG_MINTER_LCD");
    std::env::remove_var("DREGG_MINTER_ADDR");
    std::env::remove_var("DREGG_MINTER_FUNDED");
    std::env::set_var(
        "DREGG_MINTER_BALANCE",
        checkout::native_needed_for_nfts(1_000).to_string(),
    );
    let mut cell = Cell::new();
    assert_eq!(
        cell.apply(Turn::MintEntitled {
            n_deposit: [0u8; 32],
            action: action([1u8; 32]),
        })
        .unwrap_err(),
        CellError::CheckoutClosed
    );
}

#[test]
fn packet_json_has_no_rail() {
    let (_g, mut cell) = ready_cell(100);
    let d = deposit(Rail::ZecIronwood, 150_000, b"secret-txid-abcdef");
    let n = nullifier(&d);
    cell.apply(Turn::DepositSeen { opening: d }).unwrap();
    cell.apply(Turn::MintEntitled {
        n_deposit: n,
        action: action([8u8; 32]),
    })
    .unwrap();
    let packet = cell
        .apply(Turn::MintSettled {
            n_deposit: n,
            token_id: "2".into(),
            terp_tx: "lab".into(),
            receipt: *blake3::hash(b"settle-2").as_bytes(),
        })
        .unwrap()
        .unwrap();
    let json = serde_json::to_string(&packet).unwrap();
    assert!(!packet.leaks_payment_source(&json));
    for k in ["txid", "rail", "ironwood", "bitcoin", "ethereum", "secret"] {
        assert!(!json.to_ascii_lowercase().contains(k), "{k} in {json}");
    }
}

#[test]
fn sold_out_query_refunds_remaining_depositors() {
    let (_g, mut cell) = ready_cell(1);
    let d1 = deposit_salt(Rail::Eth, 1_000_000_000_000_000, b"eth-vault-log-a", 1);
    let d2 = deposit_salt(Rail::Btc, 10_000, b"btc-outpoint-b", 2);
    let n1 = nullifier(&d1);
    let n2 = nullifier(&d2);
    cell.apply(Turn::DepositSeen { opening: d1 }).unwrap();
    cell.apply(Turn::DepositSeen { opening: d2 }).unwrap();
    cell.apply(Turn::MintEntitled {
        n_deposit: n1,
        action: action([1u8; 32]),
    })
    .unwrap();
    cell.apply(Turn::MintEntitled {
        n_deposit: n2,
        action: action([2u8; 32]),
    })
    .unwrap();
    cell.apply(Turn::MintSettled {
        n_deposit: n1,
        token_id: "1".into(),
        terp_tx: "ok".into(),
        receipt: *blake3::hash(b"settle-n1").as_bytes(),
    })
    .unwrap();
    assert_eq!(
        cell.apply(Turn::MintSettled {
            n_deposit: n2,
            token_id: "2".into(),
            terp_tx: "no".into(),
            receipt: *blake3::hash(b"settle-ok").as_bytes(),
        }),
        Err(CellError::SoldOut)
    );
    let q = cell.query_sold_out_refunds("svg");
    assert!(q.sold_out);
    assert_eq!(q.minted, 1);
    assert_eq!(q.max_supply, 1);
    assert_eq!(q.refund_nullifiers.len(), 1);
    assert!(q.refund_nullifiers.contains(&n2));
    cell.apply(Turn::DepositRefunded { n_deposit: n2 })
        .unwrap();
    assert_eq!(cell.phase(&n2), Some(Phase::Refunded));
}

#[test]
fn native_mint_observe_refunds_all_rails() {
    let (_g, mut cell) = ready_cell(1);
    let rails = [
        (Rail::ZecIronwood, 150_000u128, b"ironwood-note-z".as_slice(), 1u8),
        (Rail::Eth, 1_000_000_000_000_000, b"eth-vault-log-e".as_slice(), 2),
        (Rail::Btc, 10_000, b"btc-outpoint-b".as_slice(), 3),
        (Rail::Base, 1_000_000_000_000_000, b"base-vault-log-b".as_slice(), 4),
    ];
    let mut ns = Vec::new();
    for (i, (rail, amt, raw, salt)) in rails.into_iter().enumerate() {
        let d = deposit_salt(rail, amt, raw, salt);
        let n = nullifier(&d);
        cell.apply(Turn::DepositSeen { opening: d }).unwrap();
        cell.apply(Turn::MintEntitled {
            n_deposit: n,
            action: action([i as u8 + 1; 32]),
        })
        .unwrap();
        ns.push(n);
    }
    // Native payment mint filled supply in the same block — not a DREGG settle.
    cell.apply(Turn::ObserveSupply {
        collection: "svg".into(),
        minted: 1,
        max_supply: 1,
        height: 12,
        receipt: *blake3::hash(b"supply-h12").as_bytes(),
    })
    .unwrap();
    assert_eq!(
        cell.apply(Turn::MintSettled {
            n_deposit: ns[0],
            token_id: "x".into(),
            terp_tx: "dregg".into(),
            receipt: *blake3::hash(b"dregg-lost").as_bytes(),
        }),
        Err(CellError::SoldOut)
    );
    let q = cell.query_and_refund_sold_out("svg").unwrap();
    assert!(q.sold_out);
    assert_eq!(q.refund_nullifiers.len(), 4);
    for n in &ns {
        assert_eq!(cell.phase(n), Some(Phase::Refunded));
    }
}

#[test]
fn base_rail_byte_distinct_from_eth() {
    let raw = b"same-vault-log-bytes";
    let eth = deposit(Rail::Eth, 1_000_000_000_000_000, raw);
    let base = deposit(Rail::Base, 1_000_000_000_000_000, raw);
    assert_ne!(Rail::Eth.as_byte(), Rail::Base.as_byte());
    assert_ne!(nullifier(&eth), nullifier(&base));
    assert_ne!(commit_deposit(&eth), commit_deposit(&base));
}

#[test]
fn dummy_receipt_refused() {
    let (_g, mut cell) = ready_cell(100);
    let d = deposit(Rail::Eth, 1_000_000_000_000_000, b"log");
    let n = nullifier(&d);
    cell.apply(Turn::DepositSeen { opening: d }).unwrap();
    cell.apply(Turn::MintEntitled {
        n_deposit: n,
        action: action([1u8; 32]),
    })
    .unwrap();
    assert_eq!(
        cell.apply(Turn::MintSettled {
            n_deposit: n,
            token_id: "1".into(),
            terp_tx: "x".into(),
            receipt: [1u8; 32],
        }),
        Err(CellError::DummyReceipt)
    );
}

#[test]
fn qc_unsigned_and_threshold() {
    use dregg::qc::{count_valid, sign, Checkpoint, QcError};
    let kp = dregg::qc::generate_operator();
    let pk = kp.public.to_bytes();
    let ck = Checkpoint::new(1, [9u8; 32], [8u8; 32], "deposit");
    assert_eq!(count_valid(&ck.digest, &[], &[pk], 1), Err(QcError::Unsigned));
    let s = sign(&kp, &ck.digest);
    assert!(count_valid(&ck.digest, &[s.clone()], &[pk], 1).is_ok());
    assert!(matches!(
        count_valid(&ck.digest, &[s], &[pk], 2),
        Err(QcError::Threshold { got: 1, need: 2 })
    ));
}

#[test]
fn durable_survives_reload() {
    use dregg::store::Durable;
    let dir = std::env::temp_dir().join(format!("svg-cell-{}.json", std::process::id()));
    let d = deposit(Rail::Btc, 10_000, b"outpoint-real");
    let n = nullifier(&d);
    let (_g, _) = ready_cell(100);
    {
        let (store, mut cell, books, pending, hd, bogo) = Durable::open(&dir).unwrap();
        cell.apply(Turn::SetCollection {
            id: "svg".into(),
            max_supply: 100,
        })
        .unwrap();
        cell.apply(Turn::DepositSeen { opening: d }).unwrap();
        store.save(&cell, &books, &pending, hd, bogo).unwrap();
    }
    let (_store, cell, _books, _, _, _) = Durable::open(&dir).unwrap();
    assert_eq!(cell.phase(&n), Some(Phase::Deposited));
    let _ = std::fs::remove_file(&dir);
}

#[test]
fn restart_cannot_re_entitle() {
    use dregg::store::Durable;
    let dir = std::env::temp_dir().join(format!("svg-cell-re-{}.json", std::process::id()));
    let d = deposit(Rail::Eth, 1_000_000_000_000_000, b"vault-log-eth1");
    let n = nullifier(&d);
    let (_g, _) = ready_cell(100);
    {
        let (store, mut cell, books, pending, hd, bogo) = Durable::open(&dir).unwrap();
        cell.apply(Turn::SetCollection {
            id: "svg".into(),
            max_supply: 100,
        })
        .unwrap();
        cell.apply(Turn::DepositSeen { opening: d }).unwrap();
        cell.apply(Turn::MintEntitled {
            n_deposit: n,
            action: action([1u8; 32]),
        })
        .unwrap();
        cell.apply(Turn::MintSettled {
            n_deposit: n,
            token_id: "1".into(),
            terp_tx: "ok".into(),
            receipt: *blake3::hash(b"settle-restart").as_bytes(),
        })
        .unwrap();
        store.save(&cell, &books, &pending, hd, bogo).unwrap();
    }
    let (_store, mut cell, _b, _, _, _) = Durable::open(&dir).unwrap();
    assert_eq!(
        cell.apply(Turn::MintEntitled {
            n_deposit: n,
            action: action([2u8; 32]),
        }),
        Err(CellError::DoubleSpend)
    );
    let _ = std::fs::remove_file(&dir);
}

#[test]
fn rail_payout_xor() {
    use dregg::payout::{RailBooks, RailPayout};
    let mut b = RailBooks::default();
    let n = [9u8; 32];
    b.apply(n, RailPayout::Refund, b"eth-vault-refund-txid-1".to_vec())
        .unwrap();
    assert_eq!(
        b.apply(n, RailPayout::Release, b"eth-vault-release-txid-1".to_vec()),
        Err(dregg::payout::PayoutError::Xor)
    );
}

#[test]
fn dummy_observe_refused() {
    let mut cell = Cell::new();
    assert_eq!(
        cell.apply(Turn::ObserveSupply {
            collection: "svg".into(),
            minted: 1,
            max_supply: 1,
            height: 1,
            receipt: [1u8; 32],
        }),
        Err(CellError::DummyReceipt)
    );
}

#[test]
fn mentor_bind_true_and_swapped_action_false() {
    let d = deposit(Rail::Eth, 1_000_000_000_000_000, b"log");
    let a = action([11u8; 32]);
    let c_d = commit_deposit(&d);
    let c_a = commit_action(&a);
    let b = bind(&c_d, &c_a, &a.session);
    let open = MentorOpening {
        deposit: d.clone(),
        action: a.clone(),
    };
    assert!(Mentor::verify_binding(&c_d, &c_a, &b, &open));

    let mut swapped = a.clone();
    swapped.svg_hash = [99u8; 32];
    let open_bad = MentorOpening {
        deposit: d,
        action: swapped,
    };
    assert!(!Mentor::verify_binding(&c_d, &c_a, &b, &open_bad));
}
