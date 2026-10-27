// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Classified failures returned by the coverage engine.
use thiserror::Error;
/// Failure categories mapped to the shared CLI exit protocol.
#[derive(Debug, Error)]
pub enum CoverageError {
    /// The caller cancelled execution; all child processes were terminated.
    #[error("coverage cancelled")]
    Cancelled,
    /// Invalid configuration, provenance, or coverage input.
    #[error("{0}")]
    Invalid(String),
    /// Executed test, build, or percentage gate failed.
    #[error("{0}")]
    Failed(String),
    /// A filesystem or process operation could not complete.
    #[error("{0}")]
    Infrastructure(String),
}
impl CoverageError {
    /// Returns the protocol exit code for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Cancelled => 130,
            Self::Invalid(_) => 2,
            Self::Failed(_) => 1,
            Self::Infrastructure(_) => 3,
        }
    }
}
impl From<std::io::Error> for CoverageError {
    fn from(e: std::io::Error) -> Self {
        Self::Infrastructure(e.to_string())
    }
}
impl From<serde_json::Error> for CoverageError {
    fn from(e: serde_json::Error) -> Self {
        Self::Invalid(e.to_string())
    }
}
