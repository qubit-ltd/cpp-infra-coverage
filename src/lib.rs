// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! LLVM source coverage with verified build provenance and strict quality
//! gates.
mod application;
mod cancellation;
mod cli;
mod config;
mod coverage;
pub mod coverage_error;
mod digest;
mod report_paths;
mod session;

pub use application::run;
