// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Command line entrypoint for validated LLVM coverage collection.
fn main() {
    std::process::exit(qubit_cpp_infra_coverage::run());
}
