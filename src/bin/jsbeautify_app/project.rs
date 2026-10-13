//! `jsbeautify -d --project <DIR> -o <OUTDIR>`: see [`jsbeautify::project`].
//!
//! The index and the plan are built in this process; every file is then
//! deobfuscated by a child `jsbeautify` (`--project-job`), so each file has
//! its own log and a failing file cannot take the others down.

use super::BeautifyArgs;
use super::runner::{Fallible, options_for, read_input};
use jsbeautify::project::{
    IndexedFile, LinkReport, PlanStats, ProjectSeeds, check_links, index_project, is_javascript, list_files,
    parallel_map, plan_project,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Serialize, Deserialize)]
struct Job {
    input: PathBuf,
    output: PathBuf,
    seeds: ProjectSeeds,
}

/// One file of a project, run by the child process.
pub(super) fn run_job(b: &BeautifyArgs, job: &str) -> Fallible {
    let job: Job = serde_json::from_str(&read_input(job)?)?;
    let input = job.input.to_string_lossy().into_owned();
    let output = job.output.to_string_lossy().into_owned();
    let config = b.format.load_config()?;
    let mut options = options_for(b, &config, &input, Some(&output))?;
    // The plan decides which export names name their locals (see
    // `run_project`); the files' own Phase 0.95 must agree with it.
    options.short_export_names = false;
    let code = fs::read_to_string(&job.input)?;
    let result = jsbeautify::beautifier::deobfuscate_with_project_seeds(&code, &options, job.seeds)?;
    fs::write(&job.output, result)?;
    Ok(())
}

#[derive(Serialize)]
struct FileReport<'a> {
    path: &'a str,
    #[serde(flatten)]
    status: &'a FileStatus,
    #[serde(skip_serializing_if = "ProjectSeeds::is_empty")]
    seeds: &'a ProjectSeeds,
    facts: &'a jsbeautify::project::FileFacts,
}

#[derive(Serialize)]
struct FileStatus {
    ok: bool,
    /// Both final re-parse gates (Phases 19.6 and 21.5) logged success.
    reparse_gates: bool,
    log: String,
}

#[derive(Serialize)]
struct ProjectReport<'a> {
    root: &'a Path,
    output: &'a Path,
    stats: &'a PlanStats,
    failed: Vec<&'a str>,
    links: &'a LinkReport,
    files: Vec<FileReport<'a>>,
}

pub(super) fn run_project(b: &BeautifyArgs, root: &Path) -> Fallible {
    let Some(out) = b.output.as_ref().map(PathBuf::from) else {
        return Err("--project needs -o <OUTDIR>".into());
    };
    if fs::canonicalize(root)? == fs::canonicalize(&out).unwrap_or_else(|_| out.clone()) {
        return Err("--project writes a copy: -o must not be the project directory".into());
    }
    let goal = |path: &Path| b.goal.parse_goal(&path.to_string_lossy());
    let naming = options_for(b, &b.format.load_config()?, "-", None)?;
    if naming.short_export_names {
        // A short export name names its local exactly when no project file
        // imports it (`plan_project`), so the flag would only add the
        // internal names chunks share, which say nothing.
        eprintln!(
            "[PROJECT] --short-export-names is ignored: short export names already name their locals when no project file imports them"
        );
    }
    let all = list_files(root)?;
    let (scripts, others): (Vec<PathBuf>, Vec<PathBuf>) = all.into_iter().partition(|p| is_javascript(p));
    eprintln!(
        "[PROJECT] Indexing {} JavaScript files under {}",
        scripts.len(),
        root.display()
    );
    let index = index_project(root, &scripts, goal, b.isolated, naming.keep_fn_names)?;
    let plan = plan_project(&index);
    log_plan(&plan.stats);

    for other in &others {
        let target = out.join(other);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(root.join(other), target)?;
    }
    let logs = out.join(".jsbeautify").join("logs");
    let exe = std::env::current_exe()?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let jobs: Vec<(usize, &PathBuf)> = scripts.iter().enumerate().collect();
    let statuses = parallel_map(&jobs, |&(file, relative)| {
        let job = Job {
            input: root.join(relative),
            output: out.join(relative),
            seeds: plan.seeds[file].clone(),
        };
        let log = logs.join(relative).with_added_extension("log");
        run_child(&exe, &args, &job, &log).unwrap_or_else(|error| FileStatus {
            ok: false,
            reparse_gates: false,
            log: format!("{}: {error}", log.display()),
        })
    });
    let failed: Vec<&str> = index
        .iter()
        .zip(&statuses)
        .filter(|(_, status)| !status.ok)
        .map(|(file, _)| file.path.as_str())
        .collect();
    for (relative, status) in scripts.iter().zip(&statuses) {
        if !status.ok {
            fs::copy(root.join(relative), out.join(relative))?;
        }
    }

    let outputs: Vec<(String, String)> = index
        .iter()
        .zip(&scripts)
        .map(|(file, relative)| Ok((file.path.clone(), fs::read_to_string(out.join(relative))?)))
        .collect::<std::io::Result<_>>()?;
    let links = check_links(&index, &outputs, goal);
    let gates = statuses.iter().filter(|s| s.reparse_gates).count();
    eprintln!(
        "[PROJECT] {} of {} files deobfuscated, {gates} passed both re-parse gates; {} failed (originals copied)",
        statuses.iter().filter(|s| s.ok).count(),
        statuses.len(),
        failed.len()
    );
    eprintln!(
        "[PROJECT] Links: {} imports checked, {} broken, {} files with changed exports; {} linked bindings share one name, {} differ",
        links.imports_checked,
        links.broken_imports.len(),
        links.changed_exports.len(),
        links.consistent_names,
        links.inconsistent_names
    );
    write_report(&out, root, &plan.stats, &index, &plan.seeds, &statuses, &failed, &links)?;
    if !failed.is_empty() || !links.broken_imports.is_empty() || !links.changed_exports.is_empty() {
        return Err(format!(
            "project check failed: {} files failed, {} broken imports, {} files with changed exports (see {})",
            failed.len(),
            links.broken_imports.len(),
            links.changed_exports.len(),
            out.join("project-index.json").display()
        )
        .into());
    }
    Ok(())
}

fn run_child(exe: &Path, args: &[std::ffi::OsString], job: &Job, log: &Path) -> std::io::Result<FileStatus> {
    if let Some(parent) = log.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = job.output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut child = Command::new(exe)
        .args(args)
        .args(["--project-job", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(fs::File::create(log)?)
        .spawn()?;
    let mut stdin = child.stdin.take().expect("child stdin is piped");
    serde_json::to_writer(&mut stdin, job)?;
    stdin.flush()?;
    drop(stdin);
    let ok = child.wait()?.success();
    let text = fs::read_to_string(log).unwrap_or_default();
    let reparse_gates = ["19.6", "21.5"]
        .iter()
        .all(|phase| text.contains(&format!("Phase {phase}: Generated output re-parse succeeded")));
    Ok(FileStatus {
        ok,
        reparse_gates,
        log: log.display().to_string(),
    })
}

fn log_plan(stats: &PlanStats) {
    eprintln!(
        "[PROJECT] {} files ({} modules, {} scripts, {} CommonJS); {} of {} named imports resolved to a binding",
        stats.files, stats.modules, stats.scripts, stats.commonjs, stats.resolved_imports, stats.named_imports
    );
    if !stats.unresolved_specifiers.is_empty() {
        eprintln!(
            "[PROJECT] {} relative module specifiers match no project file",
            stats.unresolved_specifiers.len()
        );
    }
    eprintln!(
        "[PROJECT] Runtime helpers: defined in {} files, {} import bindings are helpers of another chunk",
        stats.helper_files, stats.imported_helpers
    );
    eprintln!(
        "[PROJECT] Names: {:?} import bindings, {:?} exported locals; {} public / {} internal exports",
        stats.imports_named, stats.exports_named, stats.public_exports, stats.internal_exports
    );
    if stats.turbopack_files > 0 {
        eprintln!(
            "[PROJECT] Turbopack: {} chunks, {} mangled exports ({} named in their chunk, {} by another chunk's esmReexport), {} conflicting keys, {} modules in several chunks",
            stats.turbopack_files,
            stats.turbopack_mangled_exports,
            stats.turbopack_named_in_file,
            stats.turbopack_named_cross_file,
            stats.turbopack_conflicting_keys,
            stats.turbopack_shared_modules
        );
    }
    if stats.webpack_files > 0 {
        eprintln!(
            "[PROJECT] webpack: {} chunks with {} module factories",
            stats.webpack_files, stats.webpack_modules
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn write_report(
    out: &Path,
    root: &Path,
    stats: &PlanStats,
    index: &[IndexedFile],
    seeds: &[ProjectSeeds],
    statuses: &[FileStatus],
    failed: &[&str],
    links: &LinkReport,
) -> Fallible {
    let report = ProjectReport {
        root,
        output: out,
        stats,
        failed: failed.to_vec(),
        links,
        files: index
            .iter()
            .zip(seeds)
            .zip(statuses)
            .map(|((file, seeds), status)| FileReport {
                path: &file.path,
                status,
                seeds,
                facts: &file.facts,
            })
            .collect(),
    };
    let path = out.join("project-index.json");
    fs::write(&path, serde_json::to_string_pretty(&report)?)?;
    eprintln!("[PROJECT] Index written to {}", path.display());
    Ok(())
}
