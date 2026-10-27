// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::process::Output;

use serde_json::Value;
use tempfile::TempDir;

fn setup() -> TempDir {
    let p = tempfile::tempdir().unwrap();
    for d in ["src", "third_party", ".infra/coverage", "bin", "build/infra/coverage"] {
        fs::create_dir_all(p.path().join(d)).unwrap();
    }
    fs::write(p.path().join("third_party/vendor.cpp"), "int vendor();\n").unwrap();
    fs::write(p.path().join("src/a.cpp"), "int a(bool x) { return x ? 1 : 2; }\n").unwrap();
    fs::write(p.path().join(".infra/project.toml"),"schema=1\nkind='library'\nsource_roots=['src']\ntest_roots=[]\ngenerated_roots=[]\nthird_party_roots=['third_party']\n[profiles.coverage]\nconfigure='coverage'\nbuild='coverage'\ntest='coverage'\n").unwrap();
    fs::write(p.path().join(".infra/coverage/config.toml"),"schema=1\nprofile='coverage'\nbinary_targets=['tests','tests2']\nrequired_source_roots=['src']\nthreshold_exempt_files=[]\njobs=2\n[thresholds]\nlines=90.0\nfunctions=95.0\nbranches=80.0\n").unwrap();
    let script = r#"#!/usr/bin/python3
import os,sys,json,pathlib,hashlib,subprocess,time,fcntl
p=pathlib.Path(os.environ['FIXTURE_PROJECT']); args=sys.argv[1:]; name=pathlib.Path(sys.argv[0]).name
b=p/'build/infra/coverage'
def digest(config=False):
 h=hashlib.sha256()
 for f in sorted(p.rglob('*'),key=lambda x:x.relative_to(p).as_posix().encode()):
  rel=f.relative_to(p).as_posix()
  if f.is_symlink() or not f.is_file(): continue
  if config:
   if rel.startswith(('.infra/.tmp/','.infra/transactions/','.infra/.transactions/')):continue
   if not (rel.startswith('.infra/') or rel in ['CMakeLists.txt','CMakePresets.json','conanfile.py','conanfile.txt'] or rel.startswith('conan/')):continue
  elif rel.split('/')[0] in ['.git','.infra','build','target']:continue
  data=f.read_bytes(); path=rel.encode();h.update(len(path).to_bytes(8,'little'));h.update(path);h.update(len(data).to_bytes(8,'little'));h.update(data)
 return h.hexdigest()
if '--version' in args:
 print('LLVM version 18.1.8');sys.exit(0)
if name=='cpp-infra-verify':
 code=int(os.environ.get('FAKE_VERIFY_EXIT','0'))
 child_report={'schema':1,'tool':{'name':'cpp-infra-verify'},'status':('error' if code in [2,3] else ('failed' if code else 'passed')),'diagnostics':[]}
 pathlib.Path(args[args.index('--report')+1]).write_text(json.dumps(child_report))
 if code:sys.exit(code)
 run_context=json.loads(pathlib.Path(args[args.index('--run-context')+1]).read_text()) if '--run-context' in args else {}
 if run_context.get('build_context'):
  ctx=json.loads(pathlib.Path(run_context['build_context']).read_text());ctx['source_excluded_paths']=run_context.get('source_excluded_paths',[])
  state=json.loads((b/'.cpp-infra-build-state.json').read_text());state['source_excluded_paths']=ctx['source_excluded_paths'];(b/'.cpp-infra-build-state.json').write_text(json.dumps(state))
  child_report['data']={'reused_build':True};pathlib.Path(args[args.index('--report')+1]).write_text(json.dumps(child_report));pathlib.Path(args[args.index('--output')+1]).write_text(json.dumps(ctx));sys.exit(0)
 lock=open(b/'.cpp-infra.lock','a')
 if not os.environ.get('FAKE_SKIP_VERIFY_LOCK'):fcntl.flock(lock,fcntl.LOCK_EX)
 for n in ['tests','tests2']:
  if os.environ.get('REAL_LLVM'):subprocess.run(['/usr/bin/clang++-18','-O0','-g','-fprofile-instr-generate','-fcoverage-mapping',str(p/'src/a.cpp'),'-o',str(b/n)],check=True)
  else:(b/n).write_text('mapping-object')
 (b/'compile_commands.json').write_text(json.dumps([{'directory':str(b),'file':str(p/'src/a.cpp'),'command':'clang++-18 -O0 -fprofile-instr-generate -fcoverage-mapping -c a.cpp'}]))
 (b/'build.ninja').write_text('LINK_FLAGS = -fprofile-instr-generate\n')
 ctx={'schema':1,'project':str(p),'profile':'coverage','binary_dir':str(b),'configure_preset':'coverage','build_preset':'coverage','test_preset':'coverage','configuration':None,'source_digest':digest(),'config_digest':digest(True),'fingerprint':'fake-build-fingerprint','compiler':{'id':'Clang','path':'/usr/bin/clang++-18','version':'18.1.8'},'generator':'Ninja','compile_commands':str(b/'compile_commands.json'),'targets':[{'name':n,'type':'EXECUTABLE','artifacts':[str(b/n)],'sources':[str(p/'src/a.cpp')]} for n in ['tests','tests2']]}
 if os.environ.get('FAKE_STATIC_MISSING'):ctx['targets'].append({'name':'unlinked','type':'STATIC_LIBRARY','artifacts':[],'sources':[str(p/'src/unlinked.cpp')]})
 if os.environ.get('FAKE_CONTEXT_PATCH'):ctx.update(json.loads(os.environ['FAKE_CONTEXT_PATCH']))
 if os.environ.get('FAKE_COMMANDS'): (b/'compile_commands.json').write_text(os.environ['FAKE_COMMANDS'].replace('@PROJECT@',str(p)).replace('@BUILD@',str(b)))
 if os.environ.get('FAKE_LINK_FLAGS'): (b/'build.ninja').write_text('LINK_FLAGS = '+os.environ['FAKE_LINK_FLAGS'])
 (b/'CMakeCache.txt').write_text('CMAKE_HOME_DIRECTORY:INTERNAL='+str(p)+'\n')
 ctx['source_excluded_paths']=run_context.get('source_excluded_paths',[])
 ctx['configuration_inputs']=[{'path':str(p/'.infra/project.toml'),'sha256':hashlib.sha256((p/'.infra/project.toml').read_bytes()).hexdigest(),'kind':'project'}]
 ctx['cmake_cache_digest']=hashlib.sha256((b/'CMakeCache.txt').read_bytes()).hexdigest()
 if os.environ.get('FAKE_OMIT_EVIDENCE'):ctx.pop('configuration_inputs');ctx.pop('cmake_cache_digest')
 state={k:ctx[k] for k in ['schema','project','profile','source_digest','config_digest','fingerprint','source_excluded_paths','configuration_inputs','cmake_cache_digest'] if k in ctx}
 if os.environ.get('FAKE_STATE_MISMATCH'):state['fingerprint']='concurrently-rebuilt'
 (b/'.cpp-infra-build-state.json').write_text(json.dumps(state))
 (b/'.cpp-infra-project.json').write_text(json.dumps({'schema':1,'project':str(p)}))
 if os.environ.get('FAKE_CONFIG_RETARGET'):
  policy=p/'.infra/coverage/policy/thresholds.toml';policy.parent.mkdir(parents=True,exist_ok=True);target=b/'hidden-policy.toml';target.write_text('schema=1\n[thresholds]\n');policy.symlink_to(target)
 pathlib.Path(args[args.index('--output')+1]).write_text(json.dumps(ctx));sys.exit(0)
if name=='ctest':
 if '--show-only=json-v1' in args:print(json.dumps({'tests':[{'name':n,'properties':[{'name':'DISABLED','value':bool(os.environ.get('FAKE_ALL_DISABLED'))}]} for n in ['one','two']]}));sys.exit(0)
 if '--output-junit' in args:
  inner='<skipped/>' if os.environ.get('FAKE_ALL_SKIPPED') else ('<failure/>' if os.environ.get('FAKE_TEST_FAIL') else '')
  pathlib.Path(args[args.index('--output-junit')+1]).write_text('<testsuite>'+''.join('<testcase name="'+n+'">'+inner+'</testcase>' for n in ['one','two'])+'</testsuite>')
 if os.environ.get('FAKE_BAD_JUNIT'):pathlib.Path(args[args.index('--output-junit')+1]).write_text('<invalid')
 if os.environ.get('FAKE_CTEST_SLEEP'):time.sleep(float(os.environ['FAKE_CTEST_SLEEP']))
 if os.environ.get('REAL_LLVM'):
  children=[subprocess.Popen([str(b/n)]) for n in ['tests','tests2']]
  for child in children:assert child.wait()==0
  sys.exit(0)
 for i in [1,2]:pathlib.Path(os.environ['LLVM_PROFILE_FILE'].replace('%m',str(i)).replace('%p',str(os.getpid()))).write_text('new-profile')
 print('100% tests passed, 0 tests failed out of 2');sys.exit(int(os.environ.get('FAKE_TEST_FAIL','0')))
if name=='llvm-profdata':
 assert all('stale' not in x for x in args)
 pathlib.Path(args[args.index('-o')+1]).write_text('merged');sys.exit(0)
if name=='llvm-cov':
 if args[0]=='show':
  if os.environ.get('FAKE_MUTATE_SHOW'):(p/'src/a.cpp').write_text('int changed_during_show();\n')
  if os.environ.get('FAKE_MUTATE_CACHE'):(b/'CMakeCache.txt').write_text('changed cache')
  out=next(x.split('=',1)[1] for x in args if x.startswith('-output-dir='));pathlib.Path(out).mkdir(exist_ok=True);pathlib.Path(out,'index.html').write_text('<html>coverage</html>');sys.exit(0)
 count=int(os.environ.get('FAKE_COVERED','9'));total=int(os.environ.get('FAKE_TOTAL','10'))
 metric=lambda c,t:{'count':t,'covered':c,'percent':100*c/t if t else 0}
 summary={'lines':metric(count,total),'functions':metric(19,20),'branches':metric(8,10),'regions':metric(4,5),'instantiations':metric(100,200)}
 files=[{'filename':str(p/'src/a.cpp'),'summary':summary},{'filename':str(p/'third_party/vendor.cpp'),'summary':summary}]
 if os.environ.get('FAKE_MISSING'):files=files[1:]
 if os.environ.get('FAKE_UNMAPPED_TARGET') and '-object' not in args and str(b/'tests2') in args:files=[]
 report={'type':'llvm.coverage.json.export','version':'2.0.1','data':[{'files':files}]}
 if os.environ.get('FAKE_EXPORT_PATCH'):report.update(json.loads(os.environ['FAKE_EXPORT_PATCH'].replace('@PROJECT@',str(p))))
 if os.environ.get('FAKE_METRIC_PATCH'):files[0]['summary'].update(json.loads(os.environ['FAKE_METRIC_PATCH']))
 print(json.dumps(report));sys.exit(0)
sys.exit(2)
"#;
    for n in ["cpp-infra-verify", "ctest", "llvm-profdata", "llvm-cov"] {
        let path = p.path().join("bin").join(n);
        fs::write(&path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    p
}
fn cli(p: &TempDir, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cpp-infra-coverage"));
    c.arg("--project")
        .arg(p.path())
        .args(args)
        .env("FIXTURE_PROJECT", p.path())
        .env("CPP_INFRA_BIN_DIR", p.path().join("bin"))
        .env("CPP_INFRA_CTEST", p.path().join("bin/ctest"))
        .env("CPP_INFRA_LLVM_COV", p.path().join("bin/llvm-cov"))
        .env("CPP_INFRA_LLVM_PROFDATA", p.path().join("bin/llvm-profdata"));
    for (k, v) in envs {
        if *v == "__UNSET__" {
            c.env_remove(k);
        } else {
            c.env(k, v);
        }
    }
    c.output().unwrap()
}
fn run(p: &TempDir, envs: &[(&str, &str)]) -> (Output, Value) {
    let r = p.path().join("build/result.json");
    let o = cli(p, &["--report", r.to_str().unwrap(), "collect"], envs);
    let v = serde_json::from_slice(&fs::read(r).expect("report written")).unwrap();
    (o, v)
}
fn raw(p: &TempDir, r: &Value) -> std::path::PathBuf {
    let a = r["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "coverage-report")
        .unwrap();
    std::path::Path::new(r["report_root"].as_str().unwrap())
        .join(a["path"].as_str().unwrap())
        .canonicalize()
        .unwrap_or_else(|_| p.path().join("missing"))
}
#[test]
fn test_collect_multiple_objects_exact_boundary_and_artifacts() {
    let p = setup();
    let (o, v) = run(&p, &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(v["status"], "passed");
    assert!(v["commands"].as_array().unwrap().len() >= 6);
    let r: Value = serde_json::from_slice(&fs::read(raw(&p, &v)).unwrap()).unwrap();
    assert_eq!(r["objects"].as_array().unwrap().len(), 2);
    assert_eq!(r["files"].as_array().unwrap().len(), 1);
    assert_eq!(r["files"][0]["path"], "src/a.cpp");
    assert!(
        v["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["kind"] == "coverage-html")
    );
}
#[test]
fn test_collect_below_threshold_fails() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_COVERED", "8")]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(v["status"], "failed");
}
#[test]
fn test_zero_count_metric_fails() {
    let p = setup();
    let (o, _) = run(&p, &[("FAKE_COVERED", "0"), ("FAKE_TOTAL", "0")]);
    assert_eq!(o.status.code(), Some(2));
}
#[test]
fn test_missing_mapping_fails() {
    let p = setup();
    let (o, _) = run(&p, &[("FAKE_MISSING", "1")]);
    assert_eq!(o.status.code(), Some(2));
}
#[test]
fn test_test_failure_is_not_hidden_by_green_counts() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_TEST_FAIL", "1")]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(v["status"], "failed");
}
#[test]
fn test_changed_source_rejects_check() {
    let p = setup();
    let (o, v) = run(&p, &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let r = raw(&p, &v);
    fs::write(p.path().join("src/a.cpp"), "int a() {return 7;}\n").unwrap();
    let o = cli(&p, &["check", "--input", r.to_str().unwrap()], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("digest"));
}
#[test]
fn test_report_preserves_failed_collection_status() {
    let p = setup();
    let (_, v) = run(&p, &[("FAKE_COVERED", "8")]);
    let r = raw(&p, &v);
    let o = cli(&p, &["report", "--input", r.to_str().unwrap()], &[]);
    assert_eq!(o.status.code(), Some(1));
}
#[test]
fn test_stale_profiles_are_ignored() {
    let p = setup();
    fs::write(p.path().join("build/infra/coverage/stale.profraw"), "stale").unwrap();
    let (o, _) = run(&p, &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
#[test]
fn test_all_exempt_is_rejected() {
    let p = setup();
    let c = p.path().join(".infra/coverage/config.toml");
    fs::write(
        &c,
        fs::read_to_string(&c).unwrap().replace(
            "threshold_exempt_files=[]",
            "threshold_exempt_files=[{path='src/a.cpp',reason='Legacy migration'}]",
        ),
    )
    .unwrap();
    let (o, _) = run(&p, &[]);
    assert_eq!(o.status.code(), Some(2));
}
#[test]
fn test_version_mismatch_is_rejected() {
    let p = setup();
    let cov = p.path().join("bin/llvm-cov");
    fs::write(&cov, "#!/usr/bin/python3\nprint('LLVM version 17.0.0')\n").unwrap();
    let (o, _) = run(&p, &[]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn test_collect_above_boundary_passes() {
    let p = setup();
    let (o, _) = run(&p, &[("FAKE_COVERED", "10")]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
#[test]
fn test_rounding_does_not_relax_threshold() {
    let p = setup();
    let (o, _) = run(&p, &[("FAKE_COVERED", "899999"), ("FAKE_TOTAL", "1000000")]);
    assert_eq!(o.status.code(), Some(1));
}
#[test]
fn test_modified_object_rejects_reuse() {
    let p = setup();
    let (o, v) = run(&p, &[]);
    assert!(o.status.success());
    let input = raw(&p, &v);
    fs::write(p.path().join("build/infra/coverage/tests"), "replaced binary").unwrap();
    let o = cli(&p, &["check", "--input", input.to_str().unwrap()], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("digest"));
}
#[test]
fn test_real_llvm18_multiple_profiles_and_uncovered_branch() {
    let p = setup();
    fs::write(p.path().join("src/a.cpp"),"int choose(bool x) { return x ? 1 : 2; }\nint main(int argc, char**) { return choose(argc == 1) == 1 ? 0 : 1; }\n").unwrap();
    let cfg = p.path().join(".infra/coverage/config.toml");
    fs::write(
        &cfg,
        fs::read_to_string(&cfg)
            .unwrap()
            .replace("branches=80.0", "branches=50.0"),
    )
    .unwrap();
    let (o, v) = run(
        &p,
        &[
            ("REAL_LLVM", "1"),
            ("CPP_INFRA_LLVM_COV", "/usr/bin/llvm-cov-18"),
            ("CPP_INFRA_LLVM_PROFDATA", "/usr/bin/llvm-profdata-18"),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(v["external_versions"]["llvm-cov"].as_str().unwrap().contains("18"));
    let raw = raw(&p, &v);
    let r: Value = serde_json::from_slice(&fs::read(&raw).unwrap()).unwrap();
    let metric = &r["files"][0]["metrics"]["branches"];
    assert!(metric["covered"].as_u64().unwrap() < metric["total"].as_u64().unwrap());
    assert_eq!(fs::read_dir(raw.parent().unwrap().join("raw")).unwrap().count(), 2);
    let o = cli(&p, &["check", "--input", raw.to_str().unwrap()], &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn test_static_library_missing_implementation_mapping_is_rejected() {
    let p = setup();
    fs::write(p.path().join("src/unlinked.cpp"), "int unused() {return 5;}\n").unwrap();
    let (o, v) = run(&p, &[("FAKE_STATIC_MISSING", "1")]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        v["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("static-library")
    );
}
#[test]
fn test_timeout_stops_child_and_writes_infrastructure_error() {
    let p = setup();
    let cfg = p.path().join(".infra/coverage/config.toml");
    fs::write(&cfg, fs::read_to_string(&cfg).unwrap() + "\ntimeout_seconds=1\n").unwrap();
    let s = fs::read_to_string(&cfg)
        .unwrap()
        .replace("\ntimeout_seconds=1\n", "")
        .replacen("jobs=2", "jobs=2\ntimeout_seconds=1", 1);
    fs::write(cfg, s).unwrap();
    let (o, v) = run(&p, &[("FAKE_CTEST_SLEEP", "2")]);
    assert_eq!(o.status.code(), Some(3));
    assert!(v["diagnostics"][0]["message"].as_str().unwrap().contains("timed out"));
}
#[test]
fn test_run_context_missing_source_digest_is_rejected() {
    let p = setup();
    let (_, v) = run(&p, &[]);
    let ctx = p.path().join("build/context.json");
    let mut c = v.clone();
    c["profile"] = serde_json::json!("coverage");
    c.as_object_mut().unwrap().remove("source_digest");
    fs::write(&ctx, serde_json::to_vec(&c).unwrap()).unwrap();
    let o = cli(&p, &["--run-context", ctx.to_str().unwrap(), "plan"], &[]);
    assert_eq!(o.status.code(), Some(2));
}
#[test]
fn test_cancellation_returns_130_and_report() {
    let p = setup();
    let report = p.path().join("build/cancel.json");
    let mut c = Command::new(env!("CARGO_BIN_EXE_cpp-infra-coverage"));
    c.arg("--project")
        .arg(p.path())
        .arg("--report")
        .arg(&report)
        .arg("collect")
        .env("FIXTURE_PROJECT", p.path())
        .env("CPP_INFRA_BIN_DIR", p.path().join("bin"))
        .env("CPP_INFRA_CTEST", p.path().join("bin/ctest"))
        .env("CPP_INFRA_LLVM_COV", p.path().join("bin/llvm-cov"))
        .env("CPP_INFRA_LLVM_PROFDATA", p.path().join("bin/llvm-profdata"))
        .env("FAKE_CTEST_SLEEP", "10")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = c.spawn().unwrap();
    let mut ready = false;
    for _ in 0..200 {
        if walkdir::WalkDir::new(p.path().join("build/infra-runs"))
            .into_iter()
            .filter_map(Result::ok)
            .any(|e| e.file_name() == "command-4.stdout.log")
        {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(ready, "test command started");
    assert!(
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(child.wait().unwrap().code(), Some(130));
    let r: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(r["status"], "cancelled");
}

#[test]
fn test_prepare_relock_detects_concurrent_build_replacement() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_STATE_MISMATCH", "1")]);
    assert_eq!(o.status.code(), Some(2));
    assert!(v["diagnostics"][0]["message"].as_str().unwrap().contains("fingerprint"));
}
#[test]
fn test_every_selected_object_must_contain_mapping() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_UNMAPPED_TARGET", "1")]);
    assert_eq!(o.status.code(), Some(2));
    assert!(v["diagnostics"][0]["message"].as_str().unwrap().contains("mapping"));
}
#[test]
fn test_runtime_bundle_context_does_not_require_persisted_snapshot_identity() {
    let p = setup();
    fs::create_dir_all(p.path().join(".infra/tools")).unwrap();
    fs::write(
        p.path().join(".infra/tools/tools.lock.json"),
        "{\"bundle_id\":\"persisted-old-main\"}",
    )
    .unwrap();
    let (o, mut context) = run(&p, &[]);
    assert!(o.status.success());
    context["bundle_id"] = serde_json::json!("runtime-main-new");
    context["profile"] = serde_json::json!("coverage");
    let path = p.path().join("build/runtime-context.json");
    fs::write(&path, serde_json::to_vec(&context).unwrap()).unwrap();
    let o = cli(&p, &["--run-context", path.to_str().unwrap(), "plan"], &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
#[test]
fn test_runtime_snapshot_digest_is_required_and_validated() {
    let p = setup();
    let (o, mut context) = run(&p, &[]);
    assert!(o.status.success());
    context["profile"] = serde_json::json!("coverage");
    context["bundle_id"] = serde_json::json!("runtime-main-new");
    let snapshot = std::path::Path::new(context["report_root"].as_str().unwrap()).join("runtime-snapshot.json");
    fs::write(&snapshot, b"{}").unwrap();
    context["runtime_snapshot"] = serde_json::json!(snapshot);
    context["runtime_snapshot_digest"] = serde_json::json!("00");
    let path = p.path().join("build/runtime-context.json");
    fs::write(&path, serde_json::to_vec(&context).unwrap()).unwrap();
    let o = cli(&p, &["--run-context", path.to_str().unwrap(), "plan"], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("snapshot"));
}
#[test]
fn test_runtime_snapshot_accepts_same_binary_copy_and_rejects_tampering() {
    use sha2::Digest;
    use sha2::Sha256;
    let p = setup();
    let (o, mut context) = run(&p, &[]);
    assert!(o.status.success());
    context["profile"] = serde_json::json!("coverage");
    let root = std::path::Path::new(context["report_root"].as_str().unwrap()).to_owned();
    let copy = root.join("coverage-copy");
    let binary = fs::read(env!("CARGO_BIN_EXE_cpp-infra-coverage")).unwrap();
    fs::write(&copy, &binary).unwrap();
    let sha = format!("{:x}", Sha256::digest(&binary));
    let snapshot = root.join("runtime-snapshot.json");
    let identity = serde_json::json!({"schema":1,"bundle_id":"development","tools":[{"name":"cpp-infra-coverage","version":"0.1.0","revision":null,"protocol":1,"binary":copy,"executable_sha256":sha}],"offline":true,"stale":false});
    let data = serde_json::to_vec(&identity).unwrap();
    fs::write(&snapshot, &data).unwrap();
    context["runtime_snapshot"] = serde_json::json!(snapshot);
    context["runtime_snapshot_digest"] = serde_json::json!(format!("{:x}", Sha256::digest(&data)));
    let path = p.path().join("build/runtime-context.json");
    fs::write(&path, serde_json::to_vec(&context).unwrap()).unwrap();
    let o = cli(&p, &["--run-context", path.to_str().unwrap(), "plan"], &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    fs::write(copy, b"tampered").unwrap();
    let o = cli(&p, &["--run-context", path.to_str().unwrap(), "plan"], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("executable identity"));
}
#[test]
fn test_large_integer_counts_do_not_round_up_to_full_coverage() {
    let p = setup();
    let path = p.path().join(".infra/coverage/config.toml");
    fs::write(
        &path,
        fs::read_to_string(&path).unwrap().replace("lines=90.0", "lines=100.0"),
    )
    .unwrap();
    let (o, _) = run(
        &p,
        &[("FAKE_COVERED", "9007199254740992"), ("FAKE_TOTAL", "9007199254740993")],
    );
    assert_eq!(o.status.code(), Some(1));
}
#[test]
fn test_build_lock_wait_respects_timeout() {
    let p = setup();
    let path = p.path().join(".infra/coverage/config.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("jobs=2", "jobs=2\ntimeout_seconds=1"),
    )
    .unwrap();
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(p.path().join("build/infra/coverage/.cpp-infra.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let (o, v) = run(&p, &[("FAKE_SKIP_VERIFY_LOCK", "1")]);
    drop(lock);
    assert_eq!(o.status.code(), Some(3));
    assert!(v["diagnostics"][0]["message"].as_str().unwrap().contains("lock"));
}
#[test]
fn test_shared_run_context_preserves_each_coverage_call_artifacts() {
    let p = setup();
    let (o, first) = run(&p, &[]);
    assert!(o.status.success());
    let raw_first = raw(&p, &first);
    let saved = fs::read(&raw_first).unwrap();
    let mut context = first.clone();
    context["profile"] = serde_json::json!("coverage");
    let context_path = p.path().join("build/shared-context.json");
    fs::write(&context_path, serde_json::to_vec(&context).unwrap()).unwrap();
    let second_report = p.path().join("build/second-report.json");
    let o = cli(
        &p,
        &[
            "--run-context",
            context_path.to_str().unwrap(),
            "--report",
            second_report.to_str().unwrap(),
            "collect",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let second: Value = serde_json::from_slice(&fs::read(second_report).unwrap()).unwrap();
    assert_eq!(second["report_root"], first["report_root"]);
    assert_ne!(raw(&p, &second), raw_first);
    assert_eq!(fs::read(raw_first).unwrap(), saved);
}

#[test]
fn test_concurrent_shared_context_calls_keep_distinct_outputs() {
    let p = setup();
    let (o, first) = run(&p, &[]);
    assert!(o.status.success());
    let mut context = first.clone();
    context["profile"] = serde_json::json!("coverage");
    let context_path = p.path().join("build/shared-concurrent.json");
    fs::write(&context_path, serde_json::to_vec(&context).unwrap()).unwrap();
    let r1 = p.path().join("build/report-a.json");
    let r2 = p.path().join("build/report-b.json");
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            cli(
                &p,
                &[
                    "--run-context",
                    context_path.to_str().unwrap(),
                    "--report",
                    r1.to_str().unwrap(),
                    "collect",
                ],
                &[],
            )
        });
        let b = scope.spawn(|| {
            cli(
                &p,
                &[
                    "--run-context",
                    context_path.to_str().unwrap(),
                    "--report",
                    r2.to_str().unwrap(),
                    "collect",
                ],
                &[],
            )
        });
        (a.join().unwrap(), b.join().unwrap())
    });
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let a: Value = serde_json::from_slice(&fs::read(r1).unwrap()).unwrap();
    let b: Value = serde_json::from_slice(&fs::read(r2).unwrap()).unwrap();
    assert_ne!(raw(&p, &a), raw(&p, &b));
    assert_ne!(a["commands"][0]["log"], b["commands"][0]["log"]);
}
#[test]
fn test_run_context_revalidates_verified_build_without_rebuilding() {
    let p = setup();
    let (o, first) = run(&p, &[]);
    assert!(o.status.success());
    let data: Value = serde_json::from_slice(&fs::read(raw(&p, &first)).unwrap()).unwrap();
    let mut context = first.clone();
    context["profile"] = serde_json::json!("coverage");
    context["build_context"] = serde_json::json!(p.path().join(data["build_context_path"].as_str().unwrap()));
    let path = p.path().join("build/reuse-context.json");
    fs::write(&path, serde_json::to_vec(&context).unwrap()).unwrap();
    let report = p.path().join("build/reuse-report.json");
    let o = cli(
        &p,
        &[
            "--run-context",
            path.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "collect",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(
        v["commands"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["program"].as_str().unwrap().ends_with("cpp-infra-verify"))
            .count(),
        1
    );
}
#[test]
fn test_real_ctest_skip_return_code_is_not_success_even_with_profraw() {
    let p = setup();
    fs::write(p.path().join("src/a.cpp"),"int choose(bool x) { return x ? 1 : 2; }\nint main(int argc, char**) { return choose(argc == 1) == 1 ? 0 : 1; }\n").unwrap();
    fs::write(p.path().join("CMakePresets.json"),r#"{"version":6,"configurePresets":[{"name":"coverage","generator":"Ninja","binaryDir":"${sourceDir}/build/infra/coverage"}],"testPresets":[{"name":"coverage","configurePreset":"coverage"}]}"#).unwrap();
    let testfile = format!(
        "add_test(skipped \"{}\")\nset_tests_properties(skipped PROPERTIES SKIP_RETURN_CODE 0)\n",
        p.path().join("build/infra/coverage/tests").display()
    );
    fs::write(p.path().join("build/infra/coverage/CTestTestfile.cmake"), testfile).unwrap();
    let cfg = p.path().join(".infra/coverage/config.toml");
    fs::write(
        &cfg,
        fs::read_to_string(&cfg)
            .unwrap()
            .replace("lines=90.0", "lines=0.0")
            .replace("functions=95.0", "functions=0.0")
            .replace("branches=80.0", "branches=0.0"),
    )
    .unwrap();
    let (o, v) = run(
        &p,
        &[
            ("REAL_LLVM", "1"),
            ("CPP_INFRA_CTEST", "__UNSET__"),
            ("CPP_INFRA_LLVM_COV", "__UNSET__"),
            ("CPP_INFRA_LLVM_PROFDATA", "__UNSET__"),
        ],
    );
    assert_eq!(o.status.code(), Some(1), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(v["status"], "failed");
    let raw: Value = serde_json::from_slice(&fs::read(raw(&p, &v)).unwrap()).unwrap();
    assert_eq!(raw["test_summary"]["selected"], 1);
    assert_eq!(raw["test_summary"]["executed"], 0);
    assert_eq!(raw["test_summary"]["skipped"], 1);
    assert_eq!(raw["test_summary"]["passed"], false);
}
#[test]
fn test_repeated_check_excludes_its_own_project_root_report() {
    let p = setup();
    let (o, v) = run(&p, &[]);
    assert!(o.status.success());
    let input = raw(&p, &v);
    let report = p.path().join("coverage-result.json");
    for _ in 0..2 {
        let o = cli(
            &p,
            &[
                "--report",
                report.to_str().unwrap(),
                "check",
                "--input",
                input.to_str().unwrap(),
            ],
            &[],
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}

#[test]
fn test_verify_configuration_and_infrastructure_exit_codes_are_preserved() {
    for code in ["2", "3"] {
        let p = setup();
        let (o, v) = run(&p, &[("FAKE_VERIFY_EXIT", code)]);
        assert_eq!(o.status.code(), Some(code.parse().unwrap()));
        assert_eq!(v["status"], "error");
        assert!(
            v["diagnostics"][0]["message"]
                .as_str()
                .unwrap()
                .contains(&format!("exited {code}"))
        );
    }
}
#[test]
fn test_disabled_ctest_selection_runs_no_test_process() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_ALL_DISABLED", "1")]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(v["status"], "failed");
    assert!(v["commands"].as_array().unwrap().iter().all(|command| {
        !command["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "--output-junit")
    }));
}
#[test]
fn test_all_skipped_summary_stays_failed_in_check_and_report() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_ALL_SKIPPED", "1")]);
    assert_eq!(o.status.code(), Some(1));
    let input = raw(&p, &v);
    for command in ["check", "report"] {
        let o = cli(&p, &[command, "--input", input.to_str().unwrap()], &[]);
        assert_eq!(o.status.code(), Some(1));
    }
}

#[test]
fn test_fresh_rejects_configuration_retargeted_into_digest_exclusion() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_CONFIG_RETARGET", "1")]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        v["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("configuration input")
    );
}
#[test]
fn test_raw_exclusions_cannot_hide_protected_source_files() {
    let p = setup();
    fs::write(p.path().join("src/business.json"), "{}").unwrap();
    let (o, v) = run(&p, &[]);
    assert!(o.status.success());
    let input = raw(&p, &v);
    let mut data: Value = serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
    data["source_excluded_paths"] = serde_json::json!(["src/business.json"]);
    fs::write(&input, serde_json::to_vec(&data).unwrap()).unwrap();
    let o = cli(&p, &["check", "--input", input.to_str().unwrap()], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("protected project input"));
}

#[test]
fn test_collect_rejects_source_mutation_during_render_without_green_outputs() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_MUTATE_SHOW", "1")]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(v["status"], "error");
    assert_ne!(v["data"]["passed"], true);
    for artifact in v["artifacts"].as_array().unwrap() {
        if artifact["kind"] == "coverage-badge" {
            let badge: Value = serde_json::from_slice(
                &fs::read(
                    std::path::Path::new(v["report_root"].as_str().unwrap()).join(artifact["path"].as_str().unwrap()),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(badge["color"], "red");
        }
    }
}
#[test]
fn test_report_rejects_source_mutation_during_render_without_green_outputs() {
    let p = setup();
    let (o, v) = run(&p, &[]);
    assert!(o.status.success());
    let input = raw(&p, &v);
    let report = p.path().join("build/render-mutation.json");
    let o = cli(
        &p,
        &[
            "--report",
            report.to_str().unwrap(),
            "report",
            "--input",
            input.to_str().unwrap(),
        ],
        &[("FAKE_MUTATE_SHOW", "1")],
    );
    assert_eq!(o.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(v["status"], "error");
    assert_ne!(v["data"]["passed"], true);
}
#[test]
fn test_prepare_rejects_malformed_target_protocol_and_unsupported_builds() {
    for patch in [
        r#"{"targets":null}"#,
        r#"{"fingerprint":null}"#,
        r#"{"targets":[{"name":"tests","type":"STATIC_LIBRARY"}]}"#,
        r#"{"targets":[{"name":"tests","type":"EXECUTABLE","artifacts":null}]}"#,
        r#"{"targets":[{"name":"tests","type":"EXECUTABLE","artifacts":[17]}]}"#,
        r#"{"generator":"Unix Makefiles"}"#,
        r#"{"compiler":{"id":"GCC","version":"18.1.0"}}"#,
    ] {
        let p = setup();
        let (o, v) = run(&p, &[("FAKE_CONTEXT_PATCH", patch)]);
        assert_eq!(
            o.status.code(),
            Some(2),
            "{patch}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert_eq!(v["status"], "error");
    }
}
#[test]
fn test_compile_context_rejects_missing_instrumentation_and_accepts_argv() {
    for commands in [
        "{}",
        "[]",
        r#"[{"file":"@PROJECT@/src/a.cpp","directory":"@PROJECT@/src","command":"clang++ -O0 -c a.cpp"}]"#,
        r#"[{"file":"@PROJECT@/src/a.cpp","directory":"@PROJECT@/src","arguments":["clang++","-O3","-fprofile-instr-generate","-fcoverage-mapping"]}]"#,
    ] {
        let p = setup();
        let (o, _) = run(&p, &[("FAKE_COMMANDS", commands)]);
        assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let p = setup();
    let commands = r#"[{"file":"a.cpp","directory":"@PROJECT@/src","arguments":["clang++","-O0","-fprofile-instr-generate","-fcoverage-mapping"]}]"#;
    let (o, _) = run(&p, &[("FAKE_COMMANDS", commands)]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let (o, _) = run(&p, &[("FAKE_LINK_FLAGS", "-O0")]);
    assert_eq!(o.status.code(), Some(2));
}
#[test]
fn test_llvm_export_rejects_malformed_datasets_and_counter_types() {
    for patch in [
        r#"{"type":"unsupported"}"#,
        r#"{"data":null}"#,
        r#"{"data":[]}"#,
        r#"{"data":[{"files":null}]}"#,
        r#"{"data":[{"files":[{"filename":"@PROJECT@/src/missing.cpp"}]}]}"#,
    ] {
        let p = setup();
        let (o, v) = run(&p, &[("FAKE_EXPORT_PATCH", patch)]);
        assert_eq!(
            o.status.code(),
            Some(2),
            "{patch}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert_eq!(v["status"], "error");
    }
    for patch in [
        r#"{"lines":{"count":"bad","covered":9}}"#,
        r#"{"lines":{"count":10,"covered":null}}"#,
        r#"{"lines":{"count":1,"covered":2}}"#,
        r#"{"functions":null}"#,
    ] {
        let p = setup();
        let (o, _) = run(&p, &[("FAKE_METRIC_PATCH", patch)]);
        assert_eq!(o.status.code(), Some(2));
    }
}
#[test]
fn test_raw_counter_errors_and_overflow_are_not_green() {
    let p = setup();
    fs::write(p.path().join("src/b.cpp"), "int b();").unwrap();
    let (o, v) = run(&p, &[]);
    assert!(o.status.success());
    let original: Value = serde_json::from_slice(&fs::read(raw(&p, &v)).unwrap()).unwrap();
    for (covered, total) in [
        (serde_json::json!("bad"), serde_json::json!(10)),
        (serde_json::json!(9), serde_json::json!("bad")),
        (serde_json::json!(11), serde_json::json!(10)),
    ] {
        let mut value = original.clone();
        value["files"][0]["metrics"]["lines"] = serde_json::json!({"covered":covered,"total":total});
        let path = p.path().join("build/raw-invalid-counter.json");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let o = cli(&p, &["check", "--input", path.to_str().unwrap()], &[]);
        assert_eq!(o.status.code(), Some(2));
    }
    let mut overflow = original;
    overflow["files"][0]["metrics"]["lines"] = serde_json::json!({"covered":u64::MAX,"total":u64::MAX});
    let mut extra = overflow["files"][0].clone();
    extra["path"] = serde_json::json!("src/b.cpp");
    overflow["files"].as_array_mut().unwrap().push(extra);
    let path = p.path().join("build/raw-overflow.json");
    fs::write(&path, serde_json::to_vec(&overflow).unwrap()).unwrap();
    let o = cli(&p, &["check", "--input", path.to_str().unwrap()], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("overflow"));
}
#[test]
fn test_valid_partial_exemption_and_fractional_thresholds_keep_html_files() {
    let p = setup();
    let project = p.path().join(".infra/project.toml");
    fs::write(
        &project,
        fs::read_to_string(&project)
            .unwrap()
            .replace("source_roots=['src']", "source_roots=['src','third_party']")
            .replace("third_party_roots=['third_party']", "third_party_roots=[]"),
    )
    .unwrap();
    let path = p.path().join(".infra/coverage/config.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace(
                "threshold_exempt_files=[]",
                "threshold_exempt_files=[{path='src/a.cpp',reason='Legacy migration'}]",
            )
            .replace("lines=90.0", "lines=90.5"),
    )
    .unwrap();
    let (o, v) = run(&p, &[("FAKE_COVERED", "10")]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let value: Value = serde_json::from_slice(&fs::read(raw(&p, &v)).unwrap()).unwrap();
    assert_eq!(value["files"].as_array().unwrap().len(), 2);
    assert_eq!(v["data"]["summary"]["lines"]["total"], 10);
}
#[test]
fn test_missing_external_tool_is_an_infrastructure_error() {
    let p = setup();
    let missing = p.path().join("no-llvm-cov");
    let (o, v) = run(&p, &[("CPP_INFRA_LLVM_COV", missing.to_str().unwrap())]);
    assert_eq!(o.status.code(), Some(3));
    assert_eq!(v["status"], "error");
    assert!(v["diagnostics"][0]["message"].as_str().unwrap().contains("spawn"));
}

#[test]
fn test_runtime_snapshot_rejects_missing_tools_and_mismatched_identity() {
    use sha2::Digest;
    use sha2::Sha256;
    let p = setup();
    let (o, mut context) = run(&p, &[]);
    assert!(o.status.success());
    context["profile"] = serde_json::json!("coverage");
    let root = std::path::Path::new(context["report_root"].as_str().unwrap()).to_owned();
    let snapshot = root.join("runtime-negative.json");
    let path = p.path().join("build/runtime-negative-context.json");
    for tools in [
        serde_json::Value::Null,
        serde_json::json!([]),
        serde_json::json!([{"name":"cpp-infra-coverage","version":"99","revision":null,"protocol":1}]),
    ] {
        let data =
            serde_json::to_vec(&serde_json::json!({"schema":1,"bundle_id":"development","tools":tools})).unwrap();
        fs::write(&snapshot, &data).unwrap();
        context["runtime_snapshot"] = serde_json::json!(snapshot);
        context["runtime_snapshot_digest"] = serde_json::json!(format!("{:x}", Sha256::digest(&data)));
        fs::write(&path, serde_json::to_vec(&context).unwrap()).unwrap();
        let o = cli(&p, &["--run-context", path.to_str().unwrap(), "plan"], &[]);
        assert_eq!(o.status.code(), Some(2));
    }
}
#[test]
fn test_unparseable_tool_version_and_git_revision_metadata() {
    let p = setup();
    let git = p.path().join("bin/git");
    fs::write(
        &git,
        "#!/usr/bin/python3\nprint('0123456789012345678901234567890123456789')\n",
    )
    .unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
    let search = format!("{}:{}", p.path().join("bin").display(), std::env::var("PATH").unwrap());
    let (o, v) = run(&p, &[("PATH", &search)]);
    assert!(o.status.success());
    assert_eq!(v["source_revision"], "0123456789012345678901234567890123456789");
    let cov = p.path().join("bin/llvm-cov");
    fs::write(&cov, "#!/usr/bin/python3\nprint('unrecognized version')\n").unwrap();
    let (o, _) = run(&p, &[]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn test_malformed_junit_is_a_protocol_error_even_when_ctest_exit_is_zero() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_BAD_JUNIT", "1")]);
    assert_eq!(o.status.code(), Some(2));
    assert!(v["diagnostics"][0]["message"].as_str().unwrap().contains("CTest JUnit"));
}
#[test]
fn test_raw_total_counter_overflow_is_rejected_without_covered_overflow() {
    let p = setup();
    fs::write(p.path().join("src/b.cpp"), "int b();").unwrap();
    let (o, v) = run(&p, &[]);
    assert!(o.status.success());
    let mut value: Value = serde_json::from_slice(&fs::read(raw(&p, &v)).unwrap()).unwrap();
    value["files"][0]["metrics"]["lines"] = serde_json::json!({"covered":0,"total":u64::MAX});
    let mut extra = value["files"][0].clone();
    extra["path"] = serde_json::json!("src/b.cpp");
    value["files"].as_array_mut().unwrap().push(extra);
    let path = p.path().join("build/raw-total-overflow.json");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let o = cli(&p, &["check", "--input", path.to_str().unwrap()], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("overflow"));
}
#[test]
fn test_legacy_context_without_configuration_evidence_cannot_collect() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_OMIT_EVIDENCE", "1")]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(v["status"], "error");
}
#[test]
fn test_cmake_cache_changed_during_render_cannot_publish_green() {
    let p = setup();
    let (o, v) = run(&p, &[("FAKE_MUTATE_CACHE", "1")]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(v["status"], "error");
    assert_ne!(v["data"]["passed"], true);
}
