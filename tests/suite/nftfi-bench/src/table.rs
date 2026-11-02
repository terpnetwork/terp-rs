//! Markdown gas table. Mock rows are success or failure. Daemon gas stays empty
//! until a chain measurement exists.

use std::fmt::Write as _;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Row {
    pub protocol: String,
    pub action: String,
    pub mock_ok: bool,
    pub detail: String,
    pub daemon_gas: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Table {
    pub rows: Vec<Row>,
    pub notes: Vec<String>,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            notes: vec![
                "Source: in-crate harness in `nftfi-bench` (not the sibling ports).".into(),
                "Collateral: cw721-base. Mock cw-orch does not meter wasm gas.".into(),
                "Daemon `gas_used` is filled only after a broadcast on svgc-preview-1. This run does not broadcast.".into(),
            ],
        }
    }
}

impl Table {
    pub fn all_ok(&self) -> bool {
        self.rows.len() == ACTIONS.len() && self.rows.iter().all(|row| row.mock_ok)
    }

    pub fn markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# nftfi gas");
        let _ = writeln!(out);
        for note in &self.notes {
            let _ = writeln!(out, "{note}");
            let _ = writeln!(out);
        }
        let _ = writeln!(out, "| protocol | action | mock | detail | daemon gas |");
        let _ = writeln!(out, "|---|---|---|---|---|");
        for row in &self.rows {
            let mock = if row.mock_ok { "ok" } else { "err" };
            let gas = row
                .daemon_gas
                .map(|g| g.to_string())
                .unwrap_or_else(|| "—".into());
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} |",
                row.protocol, row.action, mock, row.detail, gas
            );
        }
        out
    }

    pub fn write_gas_md(&self) -> std::io::Result<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("GAS.md");
        std::fs::write(&path, self.markdown())?;
        Ok(path)
    }
}

pub const ACTIONS: &[(&str, &str)] = &[
    ("marketplace", "list"),
    ("marketplace", "bid"),
    ("marketplace", "buy"),
    ("marketplace", "accept"),
    ("marketplace", "cancel"),
    ("raffle", "create"),
    ("raffle", "buy_ticket"),
    ("raffle", "draw"),
    ("raffle", "claim"),
    ("loan", "offer"),
    ("loan", "accept"),
    ("loan", "repay"),
    ("loan", "default"),
];
