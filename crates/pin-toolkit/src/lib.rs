//! `pin-toolkit`: the multi-call front-end over every compiled plugin (D9,
//! spec §6.6). The whole CLI is [`execute`] over a [`Process`] (args, env
//! map, cwd) returning an [`Output`], so it runs in-process in tests; the
//! binary is a thin shell around [`main_with`].

pub mod cli;
pub mod config;
pub mod enable;
pub mod known;
pub mod walk;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use pin_core::run::ResolverEnv;

use crate::cli::{Cli, Mode};
use crate::config::{Effective, Fatal, env};
use crate::enable::{enable, selection};
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
    let _ = tune; // wired into the run in T21
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
    if let Err(fatal) = run_mode(&p, &cli, mode, &mut out) {
        out.stdout.clear();
        out.stderr.push_str(&format!("{fatal}\n"));
        out.code = 2;
    }
    out
}

/// Everything after parsing; a `Fatal` leaves stdout empty (T-D22).
fn run_mode(p: &Process, cli: &Cli, mode: Mode, out: &mut Output) -> Result<(), Fatal> {
    // CLI values only: env ONLY/SKIP are ignored in this mode (T-D28).
    if matches!(mode, Mode::Plugin(_)) && (cli.opts.only.is_some() || cli.opts.skip.is_some()) {
        return Err(Fatal(
            "--only/--skip cannot be combined with a plugin subcommand".into(),
        ));
    }
    let root = walk::git_root(&p.cwd).unwrap_or_else(|| p.cwd.clone());
    let eff = config::load(p, &root, cli)?;
    if mode == Mode::Plugins {
        out.stdout = plugins_listing(&eff)?;
        return Ok(());
    }
    let plan = enable(&eff, &mode)?;
    for n in &plan.notices {
        out.stderr.push_str(&format!("pin-toolkit: {n}\n"));
    }
    Ok(())
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
/// would have), then per compiled kind `  {kind}: {on|off} — {help}`.
fn plugins_listing(eff: &Effective) -> Result<String, Fatal> {
    let enabled = enable(eff, &Mode::Plugins)?.ids();
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
    Ok(out)
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
