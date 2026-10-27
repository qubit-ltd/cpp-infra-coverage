// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
use std::fs;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

fn project() -> TempDir {
    let p = tempfile::tempdir().expect("temp project");
    fs::create_dir_all(p.path().join(".infra/coverage")).unwrap();
    fs::create_dir_all(p.path().join("src")).unwrap();
    fs::write(p.path().join("src/main.cpp"), "int main() { return 0; }\n").unwrap();
    fs::write(p.path().join(".infra/project.toml"), "schema=1\nkind='application'\nsource_roots=['src']\ntest_roots=[]\ngenerated_roots=[]\nthird_party_roots=[]\n[profiles.coverage]\nconfigure='coverage'\nbuild='coverage'\ntest='coverage'\n").unwrap();
    fs::write(p.path().join(".infra/coverage/config.toml"), "schema=1\nprofile='coverage'\nbinary_targets=['tests']\nrequired_source_roots=['src']\nthreshold_exempt_files=[]\njobs=2\n[thresholds]\nlines=90.0\nfunctions=95.0\nbranches=80.0\n").unwrap();
    p
}
fn cli(p: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cpp-infra-coverage"))
        .arg("--project")
        .arg(p.path())
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn test_version_is_machine_readable() {
    let p = project();
    let o = cli(&p, &["--version", "--json"]);
    assert!(o.status.success(), "{:?}", o);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["protocol"], 1);
    assert_eq!(v["name"], "cpp-infra-coverage");
    assert!(v["revision"].is_null());
}
#[test]
fn test_plan_contains_prepare_and_no_fake_results() {
    let p = project();
    let o = cli(&p, &["plan"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["nodes"][0]["tool"], "cpp-infra-verify");
    assert!(v.to_string().contains("prepare"));
    assert!(v.to_string().contains("discovery"));
}
#[test]
fn test_unknown_config_field_is_error_and_reported() {
    let p = project();
    let c = p.path().join(".infra/coverage/config.toml");
    let mut s = fs::read_to_string(&c).unwrap();
    s.insert_str(0, "typo=true\n");
    fs::write(c, s).unwrap();
    let r = p.path().join("build/error.json");
    let o = cli(&p, &["--report", r.to_str().unwrap(), "plan"]);
    assert_eq!(o.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&fs::read(r).unwrap()).unwrap();
    assert_eq!(v["status"], "error");
    assert!(v["diagnostics"][0]["message"].as_str().unwrap().contains("typo"));
}
#[test]
fn test_source_escape_is_rejected() {
    let p = project();
    let c = p.path().join(".infra/coverage/config.toml");
    fs::write(&c, fs::read_to_string(&c).unwrap().replace("['src']", "['../']")).unwrap();
    let o = cli(&p, &["plan"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("path"));
}
#[test]
fn test_missing_requested_config_is_configuration_error() {
    let p = project();
    fs::remove_file(p.path().join(".infra/coverage/config.toml")).unwrap();
    let o = cli(&p, &["plan"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("coverage config"));
}
fn plan_report(p: &TempDir, name: &str) -> Value {
    let report = p.path().join("build").join(name);
    let o = cli(p, &["--report", report.to_str().unwrap(), "plan"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&fs::read(report).unwrap()).unwrap()
}
#[test]
fn test_config_digest_ignores_only_three_root_transaction_directories() {
    let p = project();
    let baseline = plan_report(&p, "digest-baseline.json");
    for name in [".infra/.tmp", ".infra/transactions", ".infra/.transactions"] {
        fs::create_dir_all(p.path().join(name)).unwrap();
        fs::write(p.path().join(name).join("pending.json"), "temporary").unwrap();
    }
    assert_eq!(
        plan_report(&p, "digest-temporary.json")["config_digest"],
        baseline["config_digest"]
    );
}
#[test]
fn test_config_digest_keeps_transaction_named_policy_and_nested_inputs() {
    let p = project();
    for name in [
        ".infra/.transaction-policy",
        "conan/.transaction-input",
        ".infra/policy/.transactions",
    ] {
        fs::create_dir_all(p.path().join(name)).unwrap();
        let file = p.path().join(name).join("rules.toml");
        fs::write(&file, "rule=1").unwrap();
        let old = plan_report(&p, "digest-before.json");
        fs::write(&file, "rule=2").unwrap();
        let new = plan_report(&p, "digest-after.json");
        assert_ne!(new["config_digest"], old["config_digest"], "{name} must be included");
    }
}
#[test]
fn test_session_error_report_write_failure_is_infrastructure_error() {
    let p = project();
    let context = p.path().join("invalid-context.json");
    fs::write(&context, "{}").unwrap();
    let o = cli(
        &p,
        &[
            "--run-context",
            context.to_str().unwrap(),
            "--report",
            "/dev/null/cannot-write.json",
            "plan",
        ],
    );
    assert_eq!(o.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&o.stderr).contains("write report /dev/null/cannot-write.json"));
}
#[test]
fn test_session_error_relative_report_is_resolved_against_project() {
    let p = project();
    let context = p.path().join("invalid-context.json");
    fs::write(&context, "{}").unwrap();
    let o = cli(
        &p,
        &[
            "--run-context",
            context.to_str().unwrap(),
            "--report",
            "build/context-error-relative.json",
            "plan",
        ],
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(p.path().join("build/context-error-relative.json").is_file());
}
#[test]
fn test_configuration_symlinks_to_digest_exclusions_are_rejected() {
    use std::os::unix::fs::symlink;
    for (input, target) in [
        (".infra/project.toml", "build/project.toml"),
        (".infra/coverage/config.toml", ".infra/.tmp/coverage.toml"),
        (".infra/coverage/policy/thresholds.toml", "target/thresholds.toml"),
    ] {
        let p = project();
        let input = p.path().join(input);
        let target = p.path().join(target);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let text = if input.exists() {
            fs::read_to_string(&input).unwrap()
        } else {
            "schema=1\n[thresholds]\n".into()
        };
        fs::write(&target, text).unwrap();
        if input.exists() {
            fs::remove_file(&input).unwrap();
        }
        fs::create_dir_all(input.parent().unwrap()).unwrap();
        symlink(target, &input).unwrap();
        let o = cli(&p, &["plan"]);
        assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    }
}
#[test]
fn test_posix_literal_backslash_file_is_rejected_without_digest_alias() {
    let p = project();
    fs::write(p.path().join("notes\\one.txt"), "literal path").unwrap();
    let o = cli(&p, &["plan"]);
    assert_eq!(o.status.code(), Some(2));
}
#[test]
fn test_report_output_cannot_overwrite_inputs_or_unknown_business_json() {
    let p = project();
    let business = p.path().join("business.json");
    fs::write(&business, "{\"business\":true}").unwrap();
    for output in [
        p.path().join("src/report.json"),
        p.path().join("CMakePresets.json"),
        business.clone(),
    ] {
        let o = cli(&p, &["--report", output.to_str().unwrap(), "plan"]);
        assert_eq!(o.status.code(), Some(2));
    }
    assert_eq!(fs::read_to_string(business).unwrap(), "{\"business\":true}");
    assert!(!p.path().join("src/report.json").exists());
}
#[test]
fn test_report_external_symlink_ancestor_cannot_alias_source_input() {
    use std::os::unix::fs::symlink;
    let p = project();
    let outside = tempfile::tempdir().unwrap();
    let alias = outside.path().join("source-alias");
    symlink(p.path().join("src"), &alias).unwrap();
    let output = alias.join("result.json");
    let o = cli(&p, &["--report", output.to_str().unwrap(), "plan"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(!p.path().join("src/result.json").exists());
}
#[test]
fn test_policy_directory_alias_into_build_is_rejected_but_nested_build_is_tracked() {
    use std::os::unix::fs::symlink;
    for (target, expected) in [("build/policy", 2), (".infra/policy-inputs/build", 0)] {
        let p = project();
        let target = p.path().join(target);
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("thresholds.toml"), "schema=1\n[thresholds]\n").unwrap();
        symlink(target, p.path().join(".infra/coverage/policy")).unwrap();
        let o = cli(&p, &["plan"]);
        assert_eq!(
            o.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}
#[test]
fn test_source_exclusions_require_exact_normalized_original_strings() {
    let p = project();
    fs::create_dir_all(p.path().join("reports")).unwrap();
    let mut context = plan_report(&p, "canonical-path-baseline.json");
    context["profile"] = serde_json::json!("coverage");
    let path = p.path().join("build/canonical-path-context.json");
    for alias in [
        "business.json/",
        "reports//result.json",
        "reports/./result.json",
        "./business.json",
        "business.json/../hidden.json",
        "C:business.json",
    ] {
        context["source_excluded_paths"] = serde_json::json!([alias]);
        fs::write(&path, serde_json::to_vec(&context).unwrap()).unwrap();
        let o = cli(&p, &["--run-context", path.to_str().unwrap(), "plan"]);
        assert_eq!(
            o.status.code(),
            Some(2),
            "alias accepted: {alias}; {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}
#[test]
fn test_configuration_rejects_invalid_resource_scope_and_policy_boundaries() {
    for (file, from, to) in [
        (".infra/coverage/config.toml", "schema=1", "schema=2"),
        (".infra/coverage/config.toml", "jobs=2", "jobs=0"),
        (".infra/coverage/config.toml", "profile='coverage'", "profile='missing'"),
        (
            ".infra/coverage/config.toml",
            "binary_targets=['tests']",
            "binary_targets=['tests','tests']",
        ),
        (".infra/coverage/config.toml", "lines=90.0", "lines=nan"),
        (".infra/coverage/config.toml", "branches=80.0", "unknown=80.0"),
        (".infra/project.toml", "kind='application'", "kind='unsupported'"),
        (
            ".infra/project.toml",
            "source_roots=['src']",
            "source_roots=['src/main.cpp']",
        ),
        (".infra/project.toml", "generated_roots=[]", "generated_roots=['src']"),
        (
            ".infra/project.toml",
            "generated_roots=[]",
            "generated_roots=['missing']",
        ),
        (".infra/project.toml", "configure='coverage'", "configure=''"),
    ] {
        let p = project();
        let path = p.path().join(file);
        let original = fs::read_to_string(&path).unwrap();
        assert!(original.contains(from), "fixture must match {from}");
        fs::write(path, original.replace(from, to)).unwrap();
        let o = cli(&p, &["plan"]);
        assert_eq!(o.status.code(), Some(2), "{to}: {}", String::from_utf8_lossy(&o.stderr));
    }
    for policy in [
        "{ invalid TOML",
        "schema=2\n[thresholds]\n",
        "schema=1\n[thresholds]\nlines=99.0\n",
    ] {
        let p = project();
        let dir = p.path().join(".infra/coverage/policy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("thresholds.toml"), policy).unwrap();
        let o = cli(&p, &["plan"]);
        assert_eq!(o.status.code(), Some(2));
    }
}
#[test]
fn test_malformed_context_and_project_scope_preserve_configuration_errors() {
    let p = project();
    let path = p.path().join("build/bad-context.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{").unwrap();
    let o = cli(&p, &["--run-context", path.to_str().unwrap(), "plan"]);
    assert_eq!(o.status.code(), Some(2));
    fs::write(p.path().join(".infra/project.toml"), "{ invalid TOML").unwrap();
    let o = cli(&p, &["--report", "result.json", "plan"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(!p.path().join("result.json").exists());
    let o = cli(&p, &["plan"]);
    assert_eq!(o.status.code(), Some(2));
}
#[test]
fn test_non_utf8_source_name_is_a_protocol_error() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let p = project();
    fs::write(p.path().join(OsString::from_vec(vec![b'x', 255])), "unsupported name").unwrap();
    let o = cli(&p, &["plan"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("UTF-8"));
}

#[test]
fn test_policy_directory_cannot_import_configuration_outside_project() {
    use std::os::unix::fs::symlink;
    let p = project();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("thresholds.toml"), "schema=1\n[thresholds]\n").unwrap();
    symlink(outside.path(), p.path().join(".infra/coverage/policy")).unwrap();
    let o = cli(&p, &["plan"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("configuration input escapes project"));
}
#[test]
fn test_init_manifest_delivers_project_owned_coverage_config_only_once() {
    use sha2::Digest;
    use sha2::Sha256;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: Value = serde_json::from_slice(&fs::read(root.join("conf/manifest.json")).unwrap()).unwrap();
    let entry = manifest["project_files"]
        .as_array()
        .expect("init project_files declaration")
        .iter()
        .find(|entry| entry["target"] == ".infra/coverage/config.toml")
        .expect("coverage initial config");
    assert!(
        manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["target"] != ".infra/coverage/config.toml")
    );
    assert_eq!(entry["executable"], false);
    let bytes = fs::read(root.join("conf").join(entry["source"].as_str().unwrap())).unwrap();
    assert_eq!(
        entry["sha256"],
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let config: toml::Value = toml::from_str(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(config["schema"].as_integer(), Some(1));
    assert!(!config["binary_targets"].as_array().unwrap().is_empty());
    assert!(!config["required_source_roots"].as_array().unwrap().is_empty());
}

#[test]
fn test_existing_malformed_json_is_not_overwritten_as_a_report() {
    let p = project();
    let report = p.path().join("business.json");
    fs::write(&report, "{ malformed business data").unwrap();
    let o = cli(&p, &["--report", report.to_str().unwrap(), "plan"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(fs::read_to_string(report).unwrap(), "{ malformed business data");
}
