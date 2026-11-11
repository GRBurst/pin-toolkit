//! `pin-toolkit`: the multi-call front-end over every compiled plugin (D9,
//! spec §6.6). The whole CLI is [`execute`] over a [`Process`] (args, env
//! map, cwd) returning an [`Output`], so it runs in-process in tests; the
//! binary is a thin shell around [`main_with`].

pub mod cli;
pub mod config;
pub mod enable;
pub mod known;
pub mod walk;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;

use globset::GlobSet;
use pin_core::plugin::{DynPlugin, Selection};
use pin_core::report::{ErrorEntry, Report};
use pin_core::run::{Ctx, ResolverEnv, Route, RunOptions, now_unix, run};

use crate::cli::{Cli, Mode};
use crate::config::global::HostBinding;
use crate::config::{Effective, Fatal, env};
use crate::enable::{Plan, enable, selection};
use crate::known::{KNOWN, compiled_plugin};

/// What one invocation sees of the process: read once, at the edge.
#[derive(Debug, Clone)]
pub struct Process {
    pub args: Vec<OsString>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
}

impl Process {
    /// The real process: argv, the UTF-8 environment variables (others are
    /// dropped), the current directory. The only `std::env` reader.
    pub fn from_os() -> std::io::Result<Self> {
        Ok(Self {
            args: std::env::args_os().collect(),
            env: std::env::vars_os()
                .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
                .collect(),
            cwd: std::env::current_dir()?,
        })
    }
}

/// What the process prints and its exit code.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: u8,
}

/// One invocation. `tune` adjusts the resolver env before the run (tests
/// point resolvers at fakes).
pub async fn execute(p: Process, tune: impl FnOnce(&mut ResolverEnv)) -> Output {
    let args = cli::dispatch_argv0(p.args.clone());
    let (cli, mode) = match cli::parse(&args) {
        Ok(parsed) => parsed,
        Err(e) => return usage(&e),
    };
    let mut out = Output::default();
    for var in env::unknown_vars(&p.env) {
        out.stderr.push_str(&format!(
            "pin-toolkit: ignoring unknown environment variable {var}\n"
        ));
    }
    if let Err(fatal) = run_mode(&p, &cli, mode, &mut out, tune).await {
        out.stdout.clear();
        out.stderr.push_str(&format!("{fatal}\n"));
        out.code = 2;
    }
    out
}

/// Everything after parsing: config, enablement, routing, then one pass
/// over the routes (spec §6.3) and its report: stdout the human or JSON
/// report, stderr one line per file error (T-D21), the exit code from the
/// report. A `Fatal` leaves stdout empty (T-D22).
async fn run_mode(
    p: &Process,
    cli: &Cli,
    mode: Mode,
    out: &mut Output,
    tune: impl FnOnce(&mut ResolverEnv),
) -> Result<(), Fatal> {
    // CLI values only: env ONLY/SKIP are ignored in this mode (T-D28).
    if matches!(mode, Mode::Plugin(_)) && (cli.opts.only.is_some() || cli.opts.skip.is_some()) {
        return Err(Fatal(
            "--only/--skip cannot be combined with a plugin subcommand".into(),
        ));
    }
    let root = walk::git_root(&p.cwd).unwrap_or_else(|| p.cwd.clone());
    let eff = config::load(p, &root, cli)?;
    // Before any work: an invalid `CI_SERVER_HOST` is fatal (M5d final M1).
    let mut renv = ResolverEnv::from_vars(&p.env, eff.gitlab_host.clone(), gitlab_hosts(&eff))
        .map_err(Fatal)?;
    renv.gitlab.bound = bound_tokens(&eff, &p.env);
    if mode == Mode::Plugins {
        let (listing, notices) = plugins_listing(&eff)?;
        out.stdout = listing;
        for n in &notices {
            out.stderr.push_str(&format!("pin-toolkit: {n}\n"));
        }
        return Ok(());
    }
    let plan = enable(&eff, &mode)?;
    for n in &plan.notices {
        out.stderr.push_str(&format!("pin-toolkit: {n}\n"));
    }
    let claims: Vec<(&'static str, &GlobSet)> =
        plan.plugins.iter().map(|(p, _, g)| (p.id(), g)).collect();
    let forced = match mode {
        Mode::Plugin(id) => Some(id),
        Mode::All | Mode::Plugins => None,
    };
    let found = walk::inputs(&p.cwd, &root, cli.paths());
    for w in &found.warnings {
        out.stderr.push_str(&format!("pin-toolkit: warning: {w}\n"));
    }
    let routed = walk::route(found.files, &claims, forced)?;
    tune(&mut renv);
    let ctx = Ctx::new(eff.update.clone(), now_unix(), renv);
    let routes: Vec<Route<'_>> = routed
        .routes
        .into_iter()
        .map(|(input, id)| {
            let (plugin, sel) = plugin_of(&plan, id);
            Route {
                plugin,
                sel,
                path: input.path,
                display: input.display,
            }
        })
        .collect();
    let options = RunOptions {
        dry_run: eff.dry_run,
        check: eff.check,
        concurrency: eff.concurrency,
    };
    let mut report = run(&routes, &ctx, options).await;
    record_file_errors(&mut report, routed.conflicts);
    // Walk errors have no claimant: the first enabled plugin takes them
    // (review-2 L3); without one, nothing could have been routed.
    if let Some(first) = plan.ids().first() {
        record_file_errors(
            &mut report,
            found.errors.into_iter().map(|e| (*first, e)).collect(),
        );
    }
    finish_report(&mut report, &plan.ids());
    out.stdout = if eff.json {
        report.to_json() + "\n"
    } else {
        report.render_human()
    };
    out.stderr.push_str(&error_lines(&report));
    out.code = report.exit_code(eff.check);
    Ok(())
}

/// Per-file errors found before the run: walked files claimed by 2+
/// plugins (under their first claimant, PR-L11) and walk errors (review-2
/// L3). Exit 2; the other files ran.
fn record_file_errors(report: &mut Report, errors: Vec<(&'static str, ErrorEntry)>) {
    for (id, e) in errors {
        report
            .plugins
            .entry(id.to_string())
            .or_default()
            .errors
            .push(e);
    }
}

/// The report's final shape (review-2 L7, main): every enabled plugin has
/// an entry (the JSON `plugins` keys are the enabled plugins; a seeded
/// entry is not a plugin that ran), and each plugin's errors are ordered by
/// (file, line) whatever their origin, so the JSON and stderr are
/// deterministic.
fn finish_report(report: &mut Report, enabled: &[&'static str]) {
    for id in enabled {
        report.plugins.entry(id.to_string()).or_default();
    }
    for p in report.plugins.values_mut() {
        p.errors
            .sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    }
}

/// The GitLab hosts to resolve besides the primary
/// (`plugins.gitlab-ci.gitlab-hosts`).
/// Spec §7.3: host → token for every `[hosts]` GitLab binding whose named
/// env var is set and non-empty. Other binding kinds are not GitLab's. These
/// hosts are not added to the resolve set (D45: a binding grants a
/// credential, not a lookup).
fn bound_tokens(eff: &Effective, env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    eff.hosts
        .iter()
        .filter_map(|(host, binding)| match binding {
            HostBinding::Gitlab { token_env } => env
                .get(token_env)
                .filter(|t| !t.is_empty())
                .map(|t| (host.clone(), t.clone())),
            _ => None,
        })
        .collect()
}

fn gitlab_hosts(eff: &Effective) -> BTreeSet<String> {
    eff.plugins
        .get("gitlab-ci")
        .and_then(|pl| pl.gitlab_hosts.clone())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// The enabled plugin `id` (routes name only enabled plugins) and its
/// selection.
fn plugin_of<'a>(plan: &'a Plan, id: &str) -> (&'static dyn DynPlugin, &'a Selection) {
    plan.plugins
        .iter()
        .find(|(p, ..)| p.id() == id)
        .map(|(p, sel, _)| (*p, sel))
        .expect("routes name enabled plugins")
}

/// One stderr line per file error (T-D21): `pin-toolkit: {file}[:{line}]:
/// {message}`, plugins in report order.
fn error_lines(report: &Report) -> String {
    let mut out = String::new();
    for e in report.plugins.values().flat_map(|p| &p.errors) {
        match e.line {
            Some(line) => out.push_str(&format!("pin-toolkit: {}:{line}: {}\n", e.file, e.message)),
            None => out.push_str(&format!("pin-toolkit: {}: {}\n", e.file, e.message)),
        }
    }
    out
}

/// A clap outcome that is not a parsed command line: help and version on
/// stdout (0), usage errors on stderr (2).
fn usage(e: &clap::Error) -> Output {
    let text = e.render().to_string();
    match e.kind() {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => Output {
            stdout: text,
            ..Output::default()
        },
        _ => Output {
            stderr: text,
            code: 2,
            ..Output::default()
        },
    }
}

/// `pin-toolkit plugins` (T-D27): per known plugin `{id}: {enabled |
/// disabled | not in this build}` (the state a run without a subcommand
/// would have), then per compiled kind `  {kind}: {on|off} — {help}`; and
/// the notices for stderr (an `only` id not in this build, L3).
fn plugins_listing(eff: &Effective) -> Result<(String, Vec<String>), Fatal> {
    let plan = enable(eff, &Mode::Plugins)?;
    let enabled = plan.ids();
    let mut out = String::new();
    for k in &KNOWN {
        let Some(p) = compiled_plugin(k.id) else {
            out.push_str(&format!("{}: not in this build\n", k.id));
            continue;
        };
        let state = if enabled.contains(&k.id) {
            "enabled"
        } else {
            "disabled"
        };
        out.push_str(&format!("{}: {state}\n", k.id));
        let sel = selection(eff, p);
        for kind in p.kinds() {
            let on = if sel.kinds.contains(kind.id) {
                "on"
            } else {
                "off"
            };
            out.push_str(&format!("  {}: {on} — {}\n", kind.id, kind.help));
        }
    }
    Ok((out, plan.notices))
}

/// [`execute`] on a current-thread runtime (`Ctx` is not `Send`: the run is
/// never spawned).
pub fn main_with(p: Process) -> Output {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt.block_on(execute(p, |_| {})),
        Err(e) => Output {
            stderr: format!("pin-toolkit: cannot start the runtime: {e}\n"),
            code: 2,
            ..Output::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(file: &str, line: Option<usize>, message: &str) -> ErrorEntry {
        ErrorEntry {
            file: file.into(),
            line,
            message: message.into(),
        }
    }

    #[test]
    fn walked_conflicts_are_file_errors() {
        let mut report = Report::new(vec![]);
        assert_eq!(report.exit_code(false), 0);
        let msg = "x.yml is claimed by p1 and p2 (narrow their `files` globs, or choose one: pin-toolkit <plugin> x.yml)";
        record_file_errors(&mut report, vec![("p1", entry("x.yml", None, msg))]);
        assert_eq!(report.exit_code(false), 2, "PR-L11: exit 2");
        assert_eq!(report.plugins["p1"].errors, [entry("x.yml", None, msg)]);
        assert_eq!(error_lines(&report), format!("pin-toolkit: x.yml: {msg}\n"));
    }

    /// Review-2 L7 and main: every enabled plugin has an entry; each
    /// plugin's errors are ordered by (file, line) whatever their origin
    /// (run errors, routing conflicts, walk errors), stderr likewise.
    #[test]
    fn finish_seeds_enabled_plugins_and_sorts_errors() {
        let mut report = Report::assemble(
            vec![],
            &["p1"],
            vec![
                ("p1", entry("b.yml", Some(9), "parse")),
                ("p1", entry("c.yml", None, "read")),
            ],
        );
        record_file_errors(
            &mut report,
            vec![
                ("p1", entry("b.yml", None, "conflict")),
                ("p1", entry("a/", None, "walk")),
                ("p1", entry("b.yml", Some(2), "parse")),
            ],
        );
        finish_report(&mut report, &["p1", "p2"]);
        assert_eq!(
            report
                .plugins
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["p1", "p2"]
        );
        assert!(report.plugins["p2"].errors.is_empty());
        assert_eq!(
            report.plugins["p1"].errors,
            [
                entry("a/", None, "walk"),
                entry("b.yml", None, "conflict"),
                entry("b.yml", Some(2), "parse"),
                entry("b.yml", Some(9), "parse"),
                entry("c.yml", None, "read"),
            ]
        );
        assert!(
            error_lines(&report)
                .starts_with("pin-toolkit: a/: walk\npin-toolkit: b.yml: conflict\n")
        );
        assert!(!report.ran.contains("p2"), "seeded entries did not run");
    }
}
