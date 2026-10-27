// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Command line application orchestration and common report persistence.
use std::sync::atomic::Ordering;
use std::time::Instant;

use clap::Parser;
use serde_json::json;

use crate::cancellation::CANCELLED;
use crate::cli::Action;
use crate::cli::Cli;
use crate::config;
use crate::coverage;
use crate::coverage_error::CoverageError;
use crate::report_paths;
use crate::session;
use crate::session::Session;
use crate::session::write_json;

/// Executes the CLI and returns a shared-protocol exit code.
///
/// This performs filesystem and child-process IO. Configuration, coverage
/// gates, and infrastructure failures return 2, 1 and 3 respectively, retaining
/// a report.
pub fn run() -> i32 {
    let cli = Cli::parse();
    if cli.version {
        if cli.json {
            println!(
                "{}",
                json!({"name":"cpp-infra-coverage","version":env!("CARGO_PKG_VERSION"),"revision":option_env!("CPP_INFRA_BUILD_REVISION"),"protocol":1})
            );
        } else {
            println!("cpp-infra-coverage {}", env!("CARGO_PKG_VERSION"));
        }
        return 0;
    }
    if let Err(e) = ctrlc::set_handler(|| CANCELLED.store(true, Ordering::Relaxed)) {
        eprintln!("install cancellation handler: {e}");
        return 3;
    }
    if let (Ok(project), Some(report)) = (cli.project.canonicalize(), cli.report.as_ref())
        && let Err(e) = report_paths::output(&project, report)
    {
        eprintln!("{e}");
        return e.exit_code();
    }
    let start = Instant::now();
    let mut s = match Session::new(&cli) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            if let Some(path) = &cli.report {
                let path = if path.is_absolute() {
                    path.clone()
                } else {
                    cli.project.join(path)
                };
                if let Err(write_error) = write_json(
                    &path,
                    &json!({"schema":1,"tool":{"name":"cpp-infra-coverage","version":env!("CARGO_PKG_VERSION"),"revision":option_env!("CPP_INFRA_BUILD_REVISION")},"bundle_id":null,"run_id":null,"task_id":"coverage","project":cli.project,"report_root":path.parent(),"source_revision":null,"source_digest":null,"config_digest":null,"status":"error","started_at":chrono::Utc::now().to_rfc3339(),"duration_ms":start.elapsed().as_millis(),"external_versions":{},"commands":[],"diagnostics":[{"code":"coverage-input","severity":"error","message":e.to_string(),"project_relative_path":null}],"artifacts":[]}),
                ) {
                    eprintln!("write report {}: {write_error}", path.display());
                    return 3;
                }
            }
            return e.exit_code();
        }
    };
    let mut result = (|| -> Result<(), CoverageError> {
        let (cfg, scope) = config::load(&s.project)?;
        s.timeout = cfg.timeout_seconds;
        if let Some(path) = &cli.run_context {
            let c = session::read_json(path)?;
            if c["profile"] != cfg.profile {
                return Err(session::invalid("run context profile mismatch"));
            }
        }
        match &cli.command {
            Some(Action::Plan) => {
                println!(
                    "{}",
                    json!({"schema":1,"nodes":[{"id":"prepare","tool":"cpp-infra-verify","args":["prepare","--profile",cfg.profile,"--output","<run>/build-context.json"],"profile":cfg.profile,"needs":[],"resource_locks":[],"timeout_seconds":cfg.timeout_seconds,"required":true,"output":"build-context.json"},{"id":"objects","kind":"discovery","needs":["prepare"],"description":"Discover verified File API objects and instrumented compile/link commands"},{"id":"collect","tool":"ctest + llvm-profdata + llvm-cov","needs":["objects"],"required":true,"output":"coverage/raw.json"}]})
                );
                Ok(())
            }
            Some(Action::Collect) => coverage::collect(&mut s, &cfg, &scope, cli.offline),
            Some(Action::Check { input }) => {
                let p = if input.is_absolute() {
                    input.clone()
                } else {
                    s.project.join(input)
                };
                coverage::reuse(&mut s, &cfg, &scope, &p, false, cli.offline)
            }
            Some(Action::Report { input }) => {
                let p = if input.is_absolute() {
                    input.clone()
                } else {
                    s.project.join(input)
                };
                coverage::reuse(&mut s, &cfg, &scope, &p, true, cli.offline)
            }
            None => Err(session::invalid("a subcommand is required; see --help")),
        }
    })();
    let path = cli
        .report
        .as_ref()
        .map(|p| if p.is_absolute() { p.clone() } else { s.project.join(p) })
        .unwrap_or_else(|| s.work.join("report.json"));
    if let Err(e) = report_paths::output(&s.project, &path) {
        eprintln!("refuse report replacement {}: {e}", path.display());
        return e.exit_code();
    }
    if let Err(error) = s.fresh() {
        result = match coverage::invalidate(&mut s) {
            Ok(()) => Err(error),
            Err(invalidation) => Err(invalidation),
        };
    }
    let exit = match &result {
        Ok(()) => {
            s.report["status"] = json!("passed");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            s.report["status"] = json!(if e.exit_code() == 130 {
                "cancelled"
            } else if e.exit_code() == 1 {
                "failed"
            } else {
                "error"
            });
            if let Some(d) = s.report["diagnostics"].as_array_mut() {
                d.push(json!({"code":if e.exit_code()==1{"coverage-gate"}else{"coverage-input"},"severity":"error","message":e.to_string(),"project_relative_path":null}));
            }
            e.exit_code()
        }
    };
    s.report["duration_ms"] = json!(start.elapsed().as_millis());
    if let Err(e) = write_json(&path, &s.report) {
        eprintln!("write report {}: {e}", path.display());
        return 3;
    }
    if !matches!(cli.command, Some(Action::Plan)) {
        eprintln!("coverage report: {}", path.display());
    }
    exit
}
