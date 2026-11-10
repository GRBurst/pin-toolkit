//! Command line (spec §6.6): `pin-toolkit [PATHS]…`, one subcommand per
//! known plugin, `plugins`.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Pin CI and Helm references to immutable ids.
#[derive(Debug, Parser)]
#[command(name = "pin-toolkit", version)]
pub struct Cli {
    /// Files or directories (default: the git root, else the current
    /// directory)
    pub paths: Vec<PathBuf>,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Run only the gitlab-ci plugin
    #[command(name = "gitlab-ci")]
    GitlabCi { paths: Vec<PathBuf> },
    /// Run only the helm plugin
    Helm { paths: Vec<PathBuf> },
    /// Run only the github-actions plugin
    #[command(name = "github-actions")]
    GithubActions { paths: Vec<PathBuf> },
    /// List compiled-in plugins, their kinds and enable state
    Plugins,
}

/// What a run does: every enabled plugin, one plugin, or the listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    All,
    Plugin(&'static str),
    Plugins,
}

impl Cli {
    /// The paths of the run: the subcommand's, else the top-level ones.
    pub fn paths(&self) -> &[PathBuf] {
        match &self.cmd {
            Some(Cmd::GitlabCi { paths })
            | Some(Cmd::Helm { paths })
            | Some(Cmd::GithubActions { paths }) => paths,
            Some(Cmd::Plugins) | None => &self.paths,
        }
    }

    fn mode(&self) -> Mode {
        match self.cmd {
            None => Mode::All,
            Some(Cmd::GitlabCi { .. }) => Mode::Plugin("gitlab-ci"),
            Some(Cmd::Helm { .. }) => Mode::Plugin("helm"),
            Some(Cmd::GithubActions { .. }) => Mode::Plugin("github-actions"),
            Some(Cmd::Plugins) => Mode::Plugins,
        }
    }
}

/// Parse `args` (argv[0] first).
pub fn parse(args: &[OsString]) -> Result<(Cli, Mode), clap::Error> {
    let cli = Cli::try_parse_from(args)?;
    let mode = cli.mode();
    Ok((cli, mode))
}
