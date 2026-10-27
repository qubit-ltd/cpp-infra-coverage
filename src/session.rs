// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Audited subprocesses and atomic public reports.
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use chrono::Utc;
use serde_json::Value;
use serde_json::json;

use crate::cancellation::CANCELLED;
use crate::cli::Cli;
use crate::coverage_error::CoverageError;
use crate::digest;

pub(crate) struct Session {
    pub project: PathBuf,
    pub root: PathBuf,
    pub work: PathBuf,
    pub source: String,
    pub source_excluded: Vec<String>,
    pub config: String,
    pub report: Value,
    pub timeout: u64,
    pub context: Option<PathBuf>,
    pub evidence: Option<Value>,
}
pub(crate) fn invalid(message: impl Into<String>) -> CoverageError {
    CoverageError::Invalid(message.into())
}
pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), CoverageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}
pub(crate) fn read_json(path: &Path) -> Result<Value, CoverageError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub(crate) fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, CoverageError> {
    v[key]
        .as_str()
        .ok_or_else(|| invalid(format!("missing or invalid {key}")))
}
/// Reads an array from protocol data while retaining the caller's diagnostic.
pub(crate) fn array<'a>(value: &'a Value, message: &str) -> Result<&'a [Value], CoverageError> {
    match value.as_array() {
        Some(value) => Ok(value),
        None => Err(invalid(message)),
    }
}
/// Mutates protocol array fields without assuming their JSON shape.
pub(crate) fn array_mut<'a>(value: &'a mut Value, message: &str) -> Result<&'a mut Vec<Value>, CoverageError> {
    match value.as_array_mut() {
        Some(value) => Ok(value),
        None => Err(invalid(message)),
    }
}
/// Reads a string from protocol data while retaining the caller's diagnostic.
pub(crate) fn string<'a>(value: &'a Value, message: &str) -> Result<&'a str, CoverageError> {
    match value.as_str() {
        Some(value) => Ok(value),
        None => Err(invalid(message)),
    }
}
/// Reads an integer counter from protocol data without accepting floats.
pub(crate) fn counter(value: &Value, message: &str) -> Result<u64, CoverageError> {
    match value.as_u64() {
        Some(value) => Ok(value),
        None => Err(invalid(message)),
    }
}
/// Converts filesystem names without lossy aliases, preserving diagnostics.
pub(crate) fn utf8<'a>(value: &'a OsStr, message: &str) -> Result<&'a str, CoverageError> {
    match value.to_str() {
        Some(value) => Ok(value),
        None => Err(invalid(message)),
    }
}
/// Checks component ancestry for artifact and project-relative paths.
pub(crate) fn relative<'a>(path: &'a Path, root: &Path, message: &str) -> Result<&'a Path, CoverageError> {
    match path.strip_prefix(root) {
        Ok(value) => Ok(value),
        Err(_) => Err(invalid(message)),
    }
}
pub(crate) fn tool(name: &str) -> Result<PathBuf, CoverageError> {
    if let Some(dir) = std::env::var_os("CPP_INFRA_BIN_DIR") {
        let path = PathBuf::from(dir).join(name);
        if !path.is_file() {
            return Err(CoverageError::Infrastructure(format!(
                "bundle missing tool {}",
                path.display()
            )));
        }
        Ok(path)
    } else {
        Ok(PathBuf::from(name))
    }
}
pub(crate) fn external(env: &str, fallback: &str) -> PathBuf {
    std::env::var_os(env)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fallback))
}
/// Validates optional per-run snapshot evidence without rewriting persisted
/// locks.
fn validate_runtime_snapshot(context: &Value, root: &Path) -> Result<(), CoverageError> {
    let path = context.get("runtime_snapshot");
    let expected = context.get("runtime_snapshot_digest");
    if path.is_none() && expected.is_none() {
        return Ok(());
    }
    let path = Path::new(text(context, "runtime_snapshot")?);
    if !path.is_absolute() {
        return Err(invalid("runtime snapshot path must be absolute"));
    }
    let path = path.canonicalize()?;
    if !path.starts_with(root) || digest::file(&path)? != context["runtime_snapshot_digest"] {
        return Err(invalid("runtime snapshot path/digest mismatch"));
    }
    let snapshot = read_json(&path)?;
    if snapshot["schema"] != 1 || snapshot["bundle_id"] != context["bundle_id"] {
        return Err(invalid("runtime snapshot schema/bundle mismatch"));
    }
    let tools = array(&snapshot["tools"], "runtime snapshot tools missing")?;
    let own: Vec<&Value> = tools
        .iter()
        .filter(|tool| tool["name"] == "cpp-infra-coverage")
        .collect();
    if own.len() != 1 {
        return Err(invalid("runtime snapshot requires exactly one coverage identity"));
    }
    let own = own[0];
    if own["version"] != env!("CARGO_PKG_VERSION")
        || own["revision"] != json!(option_env!("CPP_INFRA_BUILD_REVISION"))
        || own["protocol"] != 1
    {
        return Err(invalid("runtime snapshot coverage version/revision/protocol mismatch"));
    }
    let binary = Path::new(text(own, "binary")?);
    if !binary.is_absolute()
        || digest::file(binary)? != own["executable_sha256"]
        || digest::file(&std::env::current_exe()?)? != own["executable_sha256"]
    {
        return Err(invalid("runtime snapshot executable identity digest mismatch"));
    }
    Ok(())
}
impl Session {
    pub fn new(cli: &Cli) -> Result<Self, CoverageError> {
        let project = cli.project.canonicalize()?;
        if !project.is_dir() {
            return Err(invalid("project is not directory"));
        }
        crate::config::validate_inputs(&project)?;
        let mut metadata = Vec::new();
        if let Some(path) = &cli.run_context {
            metadata.push(read_json(path)?);
        }
        if let Some(crate::cli::Action::Check { input } | crate::cli::Action::Report { input }) = &cli.command {
            let input = if input.is_absolute() {
                input.clone()
            } else {
                project.join(input)
            };
            metadata.push(read_json(&input)?);
        }
        let source_excluded = crate::report_paths::merge(&project, &metadata, cli.report.as_deref())?;
        let excluded: Vec<PathBuf> = source_excluded.iter().map(|p| project.join(p)).collect();
        let references: Vec<&Path> = excluded.iter().map(PathBuf::as_path).collect();
        let source = digest::tree(&project, Some(false), &references)?;
        let config = digest::tree(&project, Some(true), &[])?;
        let mut run_id = format!("{}-{}", Utc::now().format("%Y%m%dT%H%M%S%fZ"), std::process::id());
        let mut task_id = "coverage".to_string();
        let mut bundle = "development".to_string();
        let mut root = project.join("build/infra-runs").join(&run_id);
        if let Some(context) = &cli.run_context {
            let c = read_json(context)?;
            if c["schema"] != 1 || Path::new(text(&c, "project")?).canonicalize()? != project {
                return Err(invalid("run context project/schema mismatch"));
            }
            if c["source_digest"] != source || c["config_digest"] != config {
                return Err(invalid("run context source/config digest mismatch"));
            }
            run_id = text(&c, "run_id")?.into();
            task_id = text(&c, "task_id")?.into();
            bundle = text(&c, "bundle_id")?.into();
            root = PathBuf::from(text(&c, "report_root")?);
            if !root.is_absolute() || !root.starts_with(project.join("build/infra-runs")) {
                return Err(invalid("run context report_root outside managed runs"));
            }
            if bundle.is_empty() || run_id.is_empty() || task_id.is_empty() {
                return Err(invalid("runtime bundle/run/task identity must be nonempty"));
            }
        }
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        if !root.starts_with(project.join("build/infra-runs").canonicalize()?) {
            return Err(invalid("report root symlink escape"));
        }
        if let Some(context) = &cli.run_context {
            validate_runtime_snapshot(&read_json(context)?, &root)?;
        }
        let coverage_root = root.join("coverage");
        fs::create_dir_all(&coverage_root)?;
        if !coverage_root.canonicalize()?.starts_with(&root) {
            return Err(invalid("coverage work directory symlink escape"));
        }
        let work = coverage_root.join(format!(
            "call-{}-{}",
            Utc::now().format("%Y%m%dT%H%M%S%fZ"),
            std::process::id()
        ));
        fs::create_dir(&work)?;
        let revision = if matches!(cli.command, Some(crate::cli::Action::Plan)) {
            None
        } else {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&project)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        };
        let report = json!({"schema":1,"tool":{"name":"cpp-infra-coverage","version":env!("CARGO_PKG_VERSION"),"revision":option_env!("CPP_INFRA_BUILD_REVISION")},"bundle_id":bundle,"run_id":run_id,"task_id":task_id,"project":project,"report_root":root,"source_revision":revision,"source_digest":source,"source_excluded_paths":source_excluded,"config_digest":config,"status":"error","started_at":Utc::now().to_rfc3339(),"duration_ms":0,"external_versions":{},"commands":[],"diagnostics":[],"artifacts":[],"data":{}});
        let mut forwarded = if let Some(path) = &cli.run_context {
            read_json(path)?
        } else {
            json!({"schema":1,"run_id":run_id,"task_id":task_id,"bundle_id":bundle,"project":project,"profile":crate::config::load(&project)?.0.profile,"source_digest":source,"config_digest":config,"report_root":root})
        };
        forwarded["source_excluded_paths"] = json!(source_excluded);
        let forwarded_path = work.join("run-context.json");
        write_json(&forwarded_path, &forwarded)?;
        Ok(Self {
            project,
            root,
            work,
            source,
            source_excluded,
            config,
            report,
            timeout: 300,
            context: Some(forwarded_path),
            evidence: None,
        })
    }
    pub fn fresh(&self) -> Result<(), CoverageError> {
        if let Some(context) = &self.evidence {
            crate::coverage::validate_evidence(context)?;
        }
        crate::config::validate_inputs(&self.project)?;
        crate::report_paths::merge(
            &self.project,
            &[json!({"source_excluded_paths":self.source_excluded})],
            None,
        )?;
        let paths: Vec<PathBuf> = self.source_excluded.iter().map(|p| self.project.join(p)).collect();
        let excluded: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        if digest::tree(&self.project, Some(false), &excluded)? != self.source
            || digest::tree(&self.project, Some(true), &[])? != self.config
        {
            return Err(invalid("source/config digest changed during coverage run"));
        }
        Ok(())
    }
    pub fn artifact(&mut self, kind: &str, path: &Path, required: bool) -> Result<(), CoverageError> {
        let path = path.canonicalize()?;
        let rel = relative(&path, &self.root, "artifact outside report root")?;
        let sha = if path.is_dir() {
            digest::tree(&path, None, &[])?
        } else {
            digest::file(&path)?
        };
        array_mut(&mut self.report["artifacts"], "invalid report artifacts")?
            .push(json!({"kind":kind,"path":rel,"sha256":sha,"required":required}));
        Ok(())
    }
    pub fn command(
        &mut self,
        program: &Path,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<(i32, String), CoverageError> {
        let index = self.report["commands"].as_array().map_or(0, Vec::len);
        let out_path = self.work.join(format!("command-{index}.stdout.log"));
        let err_path = self.work.join(format!("command-{index}.stderr.log"));
        let start = Instant::now();
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .envs(env)
            .stdin(Stdio::null())
            .stdout(File::create(&out_path)?)
            .stderr(File::create(&err_path)?);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| CoverageError::Infrastructure(format!("spawn {}: {e}", program.display())))?;
        let exit = loop {
            if let Some(status) = child.try_wait()? {
                break status.code().unwrap_or(1);
            }
            if CANCELLED.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(self.timeout) {
                #[cfg(unix)]
                // SAFETY: the negative PID addresses only the fresh child's dedicated process group.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                #[cfg(not(unix))]
                child.kill()?;
                child.wait()?;
                break if CANCELLED.load(Ordering::Relaxed) { 130 } else { 124 };
            }
            thread::sleep(Duration::from_millis(10));
        };
        let output = fs::read_to_string(&out_path)?;
        let stderr = fs::read_to_string(&err_path)?;
        if !stderr.is_empty() {
            eprint!("{stderr}");
        }
        array_mut(&mut self.report["commands"],"invalid report commands")?.push(json!({"program":program,"args":args,"cwd":cwd,"env":env,"exit_code":exit,"duration_ms":start.elapsed().as_millis(),"log":relative(&out_path,&self.root,"log path")?,"stderr_log":relative(&err_path,&self.root,"log path")?}));
        if exit == 130 {
            return Err(CoverageError::Cancelled);
        }
        if exit == 124 {
            return Err(CoverageError::Infrastructure(format!(
                "{} timed out",
                program.display()
            )));
        }
        Ok((exit, output))
    }
    pub fn succeed_command(
        &mut self,
        program: &Path,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<String, CoverageError> {
        let (exit, out) = self.command(program, args, cwd, env)?;
        if exit != 0 {
            return Err(CoverageError::Failed(format!("{} exited {exit}", program.display())));
        }
        Ok(out)
    }
    pub fn lock_build(&self, path: &Path) -> Result<File, CoverageError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.join(".cpp-infra.lock"))?;
        let start = Instant::now();
        loop {
            if CANCELLED.load(Ordering::Relaxed) {
                return Err(CoverageError::Cancelled);
            }
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(file),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed() > Duration::from_secs(self.timeout) {
                        return Err(CoverageError::Infrastructure("build lock timed out".into()));
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}
