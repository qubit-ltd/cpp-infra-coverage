// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Command line selection; policy always comes from project files.
use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;
#[derive(Parser)]
#[command(
    name = "cpp-infra-coverage",
    disable_version_flag = true,
    about = "Validated LLVM source-based coverage"
)]
pub(crate) struct Cli {
    #[arg(long, global = true, default_value = ".")]
    pub project: PathBuf,
    #[arg(long, global = true)]
    pub report: Option<PathBuf>,
    #[arg(long, global = true)]
    pub offline: bool,
    #[arg(long, global = true)]
    pub run_context: Option<PathBuf>,
    #[arg(long)]
    pub version: bool,
    #[arg(long)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Option<Action>,
}
#[derive(Subcommand)]
pub(crate) enum Action {
    Plan,
    Collect,
    Check {
        #[arg(long)]
        input: PathBuf,
    },
    Report {
        #[arg(long)]
        input: PathBuf,
    },
}
