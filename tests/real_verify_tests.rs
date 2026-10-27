// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
use std::fs;
use std::os::unix::fs::symlink;
use std::process::Command;

use serde_json::Value;

#[test]
#[ignore = "explicit integration requires CPP_INFRA_TEST_VERIFY and real CMake/Ninja/Clang/LLVM18"]
fn test_real_verify_parent_report_collect_twice_and_restore_raw_exclusions() {
    let p = tempfile::tempdir().unwrap();
    for d in ["src", ".infra/coverage", ".infra/verify", "bin"] {
        fs::create_dir_all(p.path().join(d)).unwrap();
    }
    fs::write(p.path().join("src/program.cpp"),"int choose(bool x) { return x ? 1 : 2; }\nint main(int argc, char**) { return choose(argc == 1) == 1 ? 0 : 1; }\n").unwrap();
    fs::write(p.path().join("CMakeLists.txt"),"cmake_minimum_required(VERSION 3.25)\nproject(CoverageFixture LANGUAGES CXX)\ninclude(CTest)\nadd_executable(program src/program.cpp)\nadd_test(NAME program COMMAND program)\n").unwrap();
    fs::write(p.path().join("CMakePresets.json"),r#"{"version":6,"configurePresets":[{"name":"coverage","generator":"Ninja","binaryDir":"${sourceDir}/build/infra/coverage","cacheVariables":{"CMAKE_BUILD_TYPE":"Debug","CMAKE_CXX_COMPILER":"/usr/bin/clang++-18","CMAKE_EXPORT_COMPILE_COMMANDS":true,"CMAKE_CXX_FLAGS":"-O0 -g -fprofile-instr-generate -fcoverage-mapping","CMAKE_EXE_LINKER_FLAGS":"-fprofile-instr-generate"}}],"buildPresets":[{"name":"coverage","configurePreset":"coverage"}],"testPresets":[{"name":"coverage","configurePreset":"coverage"}]}"#).unwrap();
    fs::write(p.path().join(".infra/project.toml"),"schema=1\nkind='application'\nsource_roots=['src']\ntest_roots=[]\ngenerated_roots=[]\nthird_party_roots=[]\n[profiles.coverage]\nconfigure='coverage'\nbuild='coverage'\ntest='coverage'\n").unwrap();
    fs::write(p.path().join(".infra/coverage/config.toml"),"schema=1\nprofile='coverage'\nbinary_targets=['program']\nrequired_source_roots=['src']\nthreshold_exempt_files=[]\njobs=1\n[thresholds]\nlines=0.0\nfunctions=0.0\nbranches=0.0\n").unwrap();
    fs::write(p.path().join(".infra/verify/config.toml"),"schema=1\nrequired_suites=['build','test']\nconfigure_timeout_seconds=120\nbuild_timeout_seconds=120\ntest_timeout_seconds=120\njobs=1\n[headers]\nroots=[]\nexclude=[]\n").unwrap();
    let verify = std::env::var_os("CPP_INFRA_TEST_VERIFY").expect("frozen real verify binary path");
    symlink(verify, p.path().join("bin/cpp-infra-verify")).unwrap();
    let report = p.path().join("result.json");
    let mut raw = None;
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_cpp-infra-coverage"))
            .arg("--project")
            .arg(p.path())
            .arg("--report")
            .arg(&report)
            .arg("collect")
            .env("CPP_INFRA_BIN_DIR", p.path().join("bin"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let data: Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
        assert_eq!(data["source_excluded_paths"], serde_json::json!(["result.json"]));
        let artifact = data["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["kind"] == "coverage-report")
            .unwrap();
        raw =
            Some(std::path::Path::new(data["report_root"].as_str().unwrap()).join(artifact["path"].as_str().unwrap()));
    }
    for command in ["check", "report"] {
        let reuse_report = p.path().join("build").join(format!("reuse-{command}.json"));
        let output = Command::new(env!("CARGO_BIN_EXE_cpp-infra-coverage"))
            .arg("--project")
            .arg(p.path())
            .arg("--report")
            .arg(&reuse_report)
            .arg(command)
            .arg("--input")
            .arg(raw.as_ref().unwrap())
            .env("CPP_INFRA_BIN_DIR", p.path().join("bin"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let result: Value = serde_json::from_slice(&fs::read(reuse_report).unwrap()).unwrap();
        let artifact = result["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|artifact| artifact["kind"] == "verify-report")
            .unwrap();
        let child: Value = serde_json::from_slice(
            &fs::read(
                std::path::Path::new(result["report_root"].as_str().unwrap()).join(artifact["path"].as_str().unwrap()),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            child["data"]["reused_build"], true,
            "validation must reuse the existing build"
        );
    }
    let metadata: Value = serde_json::from_slice(&fs::read(raw.as_ref().unwrap()).unwrap()).unwrap();
    let context: Value =
        serde_json::from_slice(&fs::read(p.path().join(metadata["build_context_path"].as_str().unwrap())).unwrap())
            .unwrap();
    assert!(!context["configuration_inputs"].as_array().unwrap().is_empty());
    assert!(context["cmake_cache_digest"].is_string());
    let cache = std::path::Path::new(context["binary_dir"].as_str().unwrap()).join("CMakeCache.txt");
    let original = fs::read(&cache).unwrap();
    let mut changed = original.clone();
    changed.extend_from_slice(b"\n# altered after coverage collection\n");
    fs::write(&cache, changed).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cpp-infra-coverage"))
        .arg("--project")
        .arg(p.path())
        .arg("check")
        .arg("--input")
        .arg(raw.as_ref().unwrap())
        .env("CPP_INFRA_BIN_DIR", p.path().join("bin"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cache digest"));
    fs::write(cache, original).unwrap();
}
