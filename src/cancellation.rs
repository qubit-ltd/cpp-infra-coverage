// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Cancellation shared by signal handling, process supervision and lock waits.
use std::sync::atomic::AtomicBool;

/// Set by the CLI signal handler; observed by child and lock supervisors.
pub(crate) static CANCELLED: AtomicBool = AtomicBool::new(false);
