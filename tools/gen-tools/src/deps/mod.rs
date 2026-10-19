//! Dependency management subsystem.
//!
//! Manages forked crate dependencies across a Cosmos monorepo with multiple
//! Rust workspace roots. Provides switching between local/git/stable modes,
//! dependency scanning, topological ordering, and dashboards.
//!
//! Source of truth is the `dependency-matrix.toml` file in `_devops/`.

pub mod discover;
pub mod graph;
pub mod matrix;
pub mod scrape;
pub mod snapshot;
pub mod switch;

use clap::Subcommand;

/// Dependency management subcommands.
#[derive(Debug, Subcommand)]
pub enum DepsCommand {
    /// Switch all forked deps to the given mode
    Switch {
        /// dep mode: stable | local | git | zk_local | zk_git | dev | zk_dev
        mode: String,
        /// Only edit specific target level: patches | ws-deps | members | all
        #[arg(long, default_value = "all")]
        target: String,
        /// Only edit this workspace (by dir name)
        #[arg(long)]
        workspace: Option<String>,
        /// Only edit this specific crate
        #[arg(long)]
        crate_name: Option<String>,
        /// Preview without writing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show current dependency mode state across all Cargo.tomls
    Status,
    /// Scan all Cargo.tomls, classify deps, report diagnostics
    Scrape {
        /// Exit non-zero on warnings (CI mode)
        #[arg(long)]
        check: bool,
        /// Only produce JSON output
        #[arg(long)]
        json: bool,
    },
    /// Generate OVERVIEW.md dashboard
    Overview {
        /// Custom output path
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Show topological push order / dependency graph
    Graph {
        /// Generate SVG visualization (requires graphviz)
        #[arg(long)]
        svg: bool,
        /// Show ordered push/update order only
        #[arg(long)]
        order: bool,
        /// Show DOT output to stdout
        #[arg(long)]
        dot: bool,
    },
    /// Run cargo update in topological order
    Update {
        /// Preview only
        #[arg(long)]
        dry_run: bool,
    },
    /// Git push all repos in topological order
    Push {
        /// git mode to use for push targets
        #[arg(long, default_value = "git")]
        mode: String,
        /// Preview only
        #[arg(long)]
        dry_run: bool,
    },
    /// Capture current workspace dep state as a baseline snapshot
    Snapshot {
        /// Output path for the snapshot file
        #[arg(short, long, default_value = "dep-snapshot.toml")]
        output: String,
        /// Generate baseline report showing drift from matrix
        #[arg(long)]
        check: bool,
    },
    /// Promote the snapshot to the dependency-matrix.toml, freezing current state
    Baseline {
        /// Path to snapshot file
        #[arg(short, long, default_value = "dep-snapshot.toml")]
        input: String,
        /// Preview without writing
        #[arg(long)]
        dry_run: bool,
    },
    /// Check current workspace against a baseline snapshot for drift
    Drift {
        /// Path to snapshot file
        #[arg(short, long, default_value = "dep-snapshot.toml")]
        baseline: String,
    },
}
