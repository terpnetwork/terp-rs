//! gen-tools: Unified CLI for CosmWasm contract type generation and dependency management.
//!
//! Usage:
//!   gen-tools                          # Run full pipeline on current directory
//!   gen-tools <workspace>              # Run on specific workspace
//!   gen-tools --config <yaml> --all    # Run on all projects in config
//!   gen-tools deps status              # Show dependency state
//!   gen-tools deps switch local        # Switch forked deps to local mode

use clap::Parser;
use gen_tools::config::{build_context, parse_steps, Cli, ProjectConfigFile, ProjectOverrides};
use gen_tools::pipeline::Pipeline;

mod deps_runner;

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    // ── Handle `list` pseudo-subcommand ────────────────────────────────
    if cli.steps == "list" {
        let pipeline = Pipeline::new(&cli.steps);
        pipeline.list_generators();
        return Ok(());
    }

    // Parse enabled/disabled steps once
    let (_enabled_steps, _disabled_steps) = parse_steps(&cli.steps);

    // ── deps subcommand ────────────────────────────────────────────────
    if let Some(cli_cmd) = &cli.command {
        match cli_cmd {
            gen_tools::config::CliSubcommand::Deps(cmd) => {
                return deps_runner::run_deps_command(cmd);
            }
        }
    }

    // ── Config mode ────────────────────────────────────────────────────
    if let Some(config_path) = &cli.config {
        let config_file = ProjectConfigFile::load(config_path)?;
        let config_dir = config_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("config path has no parent directory"))?;

        // Get the list of projects to run
        let selected: Vec<&gen_tools::config::ProjectEntry> = if let Some(ref name) = cli.project {
            let entry = config_file
                .find_by_name(name)
                .ok_or_else(|| anyhow::anyhow!("project '{}' not found in config", name))?;
            vec![entry]
        } else if cli.all {
            config_file.projects.iter().collect()
        } else {
            anyhow::bail!("use --project <name> or --all with --config");
        };

        for entry in &selected {
            let ws_root = config_file.resolve_project_path(config_dir, entry);

            let overrides = ProjectOverrides {
                steps: entry.steps.clone().unwrap_or_else(|| cli.steps.clone()),
                skip_schema: entry.skip_schema.unwrap_or(cli.skip_schema),
                proto_modules: entry
                    .proto_modules
                    .clone()
                    .unwrap_or_else(|| cli.proto_modules.clone()),
                ts_out: entry.ts_out.clone().or_else(|| cli.ts_out.clone()),
                py_out: entry.py_out.clone().or_else(|| cli.py_out.clone()),
                zod_out: entry.zod_out.clone().or_else(|| cli.zod_out.clone()),
                proto_out: entry.proto_out.clone().or_else(|| cli.proto_out.clone()),
                go_out: entry.go_out.clone().or_else(|| cli.go_out.clone()),
                rust_out: entry.rust_out.clone().or_else(|| cli.rust_out.clone()),
                openapi_out: entry.openapi_out.clone().or_else(|| cli.openapi_out.clone()),
                tz_out: entry.tz_out.clone(),
                tz_heuristics: entry.tz_heuristics.clone(),
                tz_episodes: entry.tz_episodes.clone(),
                tz_recipes_dir: entry.tz_recipes_dir.clone(),
                tz_profiles_dir: entry.tz_profiles_dir.clone(),
                unified: cli.unified,
                unified_base: if cli.unified {
                    Some(config_dir.to_path_buf())
                } else {
                    None
                },
            };

            let project_steps = entry
                .steps
                .as_deref()
                .unwrap_or(&cli.steps)
                .to_string();

            match build_context(&ws_root, &overrides, cli.output_dir.clone()) {
                Ok(ctx) => {
                    let pipeline = Pipeline::new(&project_steps);
                    let results = pipeline.run(&ctx);
                    Pipeline::print_results(&results);
                    if results.iter().any(|r| r.is_err()) {
                        anyhow::bail!("project '{}' had failing steps", entry.name);
                    }
                }
                Err(e) => {
                    anyhow::bail!("error setting up project '{}': {}", entry.name, e);
                }
            }
        }

        return Ok(());
    }

    // ── Direct mode ────────────────────────────────────────────────────
    let overrides = ProjectOverrides {
        steps: cli.steps.clone(),
        skip_schema: cli.skip_schema,
        proto_modules: cli.proto_modules.clone(),
        ts_out: cli.ts_out.clone(),
        py_out: cli.py_out.clone(),
        zod_out: cli.zod_out.clone(),
        proto_out: cli.proto_out.clone(),
        go_out: cli.go_out.clone(),
        rust_out: cli.rust_out.clone(),
        openapi_out: cli.openapi_out.clone(),
        tz_out: None,
        tz_heuristics: None,
        tz_episodes: None,
        tz_recipes_dir: None,
        tz_profiles_dir: None,
        unified: cli.unified,
        unified_base: if cli.unified {
            Some(std::env::current_dir().unwrap_or_default())
        } else {
            None
        },
    };

    let ctx = build_context(&cli.workspace_root, &overrides, cli.output_dir.clone())?;
    let pipeline = Pipeline::new(&cli.steps);
    let results = pipeline.run(&ctx);
    Pipeline::print_results(&results);
    if results.iter().any(|r| r.is_err()) {
        anyhow::bail!("pipeline had failing steps");
    }

    Ok(())
}