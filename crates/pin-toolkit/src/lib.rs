//! `pin-toolkit`: the multi-call front-end over every compiled plugin (D9,
//! spec §6.6). The whole CLI is [`execute`] over a [`Process`] (args, env
//! map, cwd) returning an [`Output`], so it runs in-process in tests; the
//! binary is a thin shell around [`main_with`].

pub mod cli;
pub mod known;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use pin_core::plugin::KindSet;
use pin_core::run::ResolverEnv;

use crate::cli::Mode;
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
    let (_cli, mode) = match cli::parse(&p.args) {
        Ok(parsed) => parsed,
        Err(e) => {
            return Output {
                stderr: e.render().to_string(),
                code: 2,
                ..Output::default()
            };
        }
    };
    match mode {
        Mode::Plugins => Output {
            stdout: plugins_listing(),
            ..Output::default()
        },
        Mode::All | Mode::Plugin(_) => Output::default(),
    }
}

/// `pin-toolkit plugins` (T-D27): per known plugin `{id}: {state}`, then
/// per compiled kind `  {kind}: {on|off} — {help}`.
fn plugins_listing() -> String {
    let mut out = String::new();
    for k in &KNOWN {
        let Some(p) = compiled_plugin(k.id) else {
            out.push_str(&format!("{}: not in this build\n", k.id));
            continue;
        };
        out.push_str(&format!("{}: enabled\n", k.id));
        let on = KindSet::defaults(p.kinds());
        for kind in p.kinds() {
            let state = if on.contains(kind.id) { "on" } else { "off" };
            out.push_str(&format!("  {}: {state} — {}\n", kind.id, kind.help));
        }
    }
    out
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
