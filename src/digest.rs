// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Shared canonical file-set digests.
use std::fs;
use std::path::Path;

use sha2::Digest;
use sha2::Sha256;
use walkdir::WalkDir;

use crate::coverage_error::CoverageError;
use crate::session::utf8;
pub(crate) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}
pub(crate) fn file(path: &Path) -> Result<String, CoverageError> {
    Ok(hash(&fs::read(path)?))
}
pub(crate) fn tree(root: &Path, config: Option<bool>, excluded: &[&Path]) -> Result<String, CoverageError> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().filter_entry(|e| {
        let rel = e.path().strip_prefix(root).unwrap_or(e.path());
        if excluded.iter().any(|x| e.path().starts_with(x)) {
            return false;
        }
        if config == Some(true)
            && e.file_type().is_dir()
            && [".infra/.tmp", ".infra/transactions", ".infra/.transactions"]
                .iter()
                .any(|path| rel == Path::new(path))
        {
            return false;
        }
        if config.is_some() {
            let first = rel.components().next().map(|x| x.as_os_str());
            if first
                .is_some_and(|x| x == ".git" || x == "build" || x == "target" || config == Some(false) && x == ".infra")
            {
                return false;
            }
        }
        true
    }) {
        let e = entry.map_err(|e| CoverageError::Infrastructure(e.to_string()))?;
        if !e.file_type().is_file() {
            continue;
        }
        let rel = e
            .path()
            .strip_prefix(root)
            .map_err(|e| CoverageError::Invalid(e.to_string()))?;
        let rel = utf8(rel.as_os_str(), "non UTF-8 path")?;
        if cfg!(unix) && rel.contains('\\') {
            return Err(CoverageError::Invalid(format!(
                "literal backslash path is unsupported: {rel}"
            )));
        }
        let rel = rel.to_owned();
        if config == Some(true)
            && !(rel.starts_with(".infra/")
                || rel.starts_with("conan/")
                || matches!(
                    rel.as_str(),
                    "CMakeLists.txt" | "CMakePresets.json" | "conanfile.py" | "conanfile.txt"
                ))
        {
            continue;
        }
        files.push((rel, e.path().to_owned()));
    }
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut h = Sha256::new();
    for (rel, path) in files {
        let bytes = fs::read(path)?;
        h.update((rel.len() as u64).to_le_bytes());
        h.update(rel.as_bytes());
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    Ok(h.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}
