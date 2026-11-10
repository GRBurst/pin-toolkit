//! Command line (spec §6.6): `pin-toolkit [OPTIONS] [PATHS]…`, one
//! subcommand per known plugin, `plugins`; `argv[0]` aliases `pin-<plugin>`
//! and `pin-ci` (D33) insert the subcommand.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use crate::config::repo::{unknown_kind, unknown_plugin};
use crate::config::{Fatal, Layer, id_list, valid_gitlab_host};
use crate::known::{ALIASES, KNOWN, known};

// No `args_conflicts_with_subcommands` (PR-H1): with it, clap stops
// recognising a subcommand after any global flag. Without it a subcommand
// is recognised only before the first path: `[a.yml, gitlab-ci, b.yml]`
// are three paths.
/// Pin CI and Helm references to immutable ids.
#[derive(Debug, Parser)]
#[command(name = "pin-toolkit", version)]
pub struct Cli {
    #[command(flatten)]
    pub opts: Opts,
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

/// The §6.6 options; each is one config field (or a CLI-only switch).
/// Flags are presence-only (T-D8).
#[derive(Debug, Args)]
pub struct Opts {
    /// Resolve and report without writing files
    #[arg(long, global = true)]
    pub dry_run: bool,
    /// --dry-run, and exit 1 if anything would change (CI gate)
    #[arg(long, global = true)]
    pub check: bool,
    /// Print the report as JSON
    #[arg(long, global = true)]
    pub json: bool,
    /// Bump versioned references to the latest compatible stable tag
    #[arg(long, global = true)]
    pub update: bool,
    /// With --update, also allow a higher major version
    #[arg(long, global = true)]
    pub update_major: bool,
    /// With --update, skip candidates younger than N days (0 = off)
    #[arg(long, global = true, value_name = "N")]
    pub min_age_days: Option<u32>,
    /// Max resolutions in flight per file (default 8)
    #[arg(long, global = true, value_name = "N")]
    pub concurrency: Option<usize>,
    /// Run only these plugins
    #[arg(long, global = true, value_delimiter = ',', value_name = "P,…")]
    pub only: Option<Vec<String>>,
    /// Do not run these plugins
    #[arg(long, global = true, value_delimiter = ',', value_name = "P,…")]
    pub skip: Option<Vec<String>>,
    /// Turn one kind on or off (repeatable)
    #[arg(long, global = true, value_name = "P:K=on|off")]
    pub kind: Vec<String>,
    /// Repository config file (default: .pin-toolkit.toml at the git root)
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Do not read a config file
    #[arg(long, global = true)]
    pub no_config: bool,
    /// Primary GitLab host (default: CI_SERVER_HOST, else gitlab.com)
    #[arg(long, global = true, value_name = "H")]
    pub gitlab_host: Option<String>,
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

/// `argv[0]` dispatch (T-D9): when the program name (basename, minus
/// `.exe`) is an alias (`pin-ci`) or `pin-<id>` for a known plugin id,
/// insert the plugin subcommand after it.
pub fn dispatch_argv0(mut args: Vec<OsString>) -> Vec<OsString> {
    let Some(name) = args
        .first()
        .and_then(|a| Path::new(a).file_name())
        .and_then(|n| n.to_str())
    else {
        return args;
    };
    let name = name.strip_suffix(".exe").unwrap_or(name);
    let id = ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, id)| *id)
        .or_else(|| {
            let id = name.strip_prefix("pin-")?;
            KNOWN.iter().find(|k| k.id == id).map(|k| k.id)
        });
    if let Some(id) = id {
        args.insert(1, id.into());
    }
    args
}

/// Parse `args` (`argv[0]` first, after [`dispatch_argv0`]).
pub fn parse(args: &[OsString]) -> Result<(Cli, Mode), clap::Error> {
    let cli = Cli::try_parse_from(args)?;
    let mode = cli.mode();
    Ok((cli, mode))
}

/// The CLI layer: presence flags → `Some(true)`, absent → `None`;
/// `--only`/`--skip` per [`id_list`]; `--kind` and `--gitlab-host`
/// validated. `--config`/`--no-config` select
/// the repo file and are no layer fields.
pub fn layer(cli: &Cli) -> Result<Layer, Fatal> {
    let o = &cli.opts;
    let flag = |b: bool| b.then_some(true);
    if let Some(h) = &o.gitlab_host
        && !valid_gitlab_host(h)
    {
        return Err(Fatal(format!("--gitlab-host: invalid GitLab host `{h}`")));
    }
    let mut layer = Layer {
        update: flag(o.update),
        update_major: flag(o.update_major),
        min_age_days: o.min_age_days,
        concurrency: o.concurrency,
        json: flag(o.json),
        dry_run: flag(o.dry_run),
        check: flag(o.check),
        only: o
            .only
            .as_ref()
            .and_then(|v| id_list(v.iter().map(String::as_str))),
        skip: o
            .skip
            .as_ref()
            .and_then(|v| id_list(v.iter().map(String::as_str))),
        gitlab_host: o.gitlab_host.clone(),
        ..Layer::default()
    };
    for v in &o.kind {
        let (id, kind, on) = parse_kind(v)?;
        layer
            .plugins
            .entry(id.to_string())
            .or_default()
            .kinds
            .insert(kind.to_string(), on);
    }
    Ok(layer)
}

/// `<plugin>:<kind>=on|off` with a known plugin and kind.
fn parse_kind(v: &str) -> Result<(&str, &str, bool), Fatal> {
    let bad = || {
        Fatal(format!(
            "--kind expects <plugin>:<kind>=on|off, got \"{v}\""
        ))
    };
    let (pk, state) = v.split_once('=').ok_or_else(bad)?;
    let (id, kind) = pk.split_once(':').ok_or_else(bad)?;
    let on = match state {
        "on" => true,
        "off" => false,
        _ => return Err(bad()),
    };
    let k = known(id).ok_or_else(|| Fatal(unknown_plugin(id)))?;
    if !k.kinds.contains(&kind) {
        return Err(Fatal(unknown_kind(id, kind)));
    }
    Ok((id, kind, on))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    fn parsed(v: &[&str]) -> (Cli, Mode) {
        let mut a = os(&["pin-toolkit"]);
        a.extend(os(v));
        parse(&a).unwrap_or_else(|e| panic!("{v:?}: {e}"))
    }

    fn paths(cli: &Cli) -> Vec<String> {
        cli.paths()
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    }

    #[test]
    fn cli_flags_map_to_layer() {
        let (cli, _) = parsed(&[]);
        assert_eq!(layer(&cli), Ok(Layer::default()), "absent → None");
        assert!(cli.opts.config.is_none() && !cli.opts.no_config);

        let rows: Vec<(&[&str], Layer)> = vec![
            (
                &["--dry-run"],
                Layer {
                    dry_run: Some(true),
                    ..Layer::default()
                },
            ),
            (
                &["--check"],
                Layer {
                    check: Some(true),
                    ..Layer::default()
                },
            ),
            (
                &["--json"],
                Layer {
                    json: Some(true),
                    ..Layer::default()
                },
            ),
            (
                &["--update"],
                Layer {
                    update: Some(true),
                    ..Layer::default()
                },
            ),
            (
                &["--update-major"],
                Layer {
                    update_major: Some(true),
                    ..Layer::default()
                },
            ),
            (
                &["--min-age-days", "7"],
                Layer {
                    min_age_days: Some(7),
                    ..Layer::default()
                },
            ),
            (
                &["--concurrency", "3"],
                Layer {
                    concurrency: Some(3),
                    ..Layer::default()
                },
            ),
            (
                &["--only", "gitlab-ci,helm"],
                Layer {
                    only: Some(vec!["gitlab-ci".into(), "helm".into()]),
                    ..Layer::default()
                },
            ),
            (
                &["--skip", "helm"],
                Layer {
                    skip: Some(vec!["helm".into()]),
                    ..Layer::default()
                },
            ),
            // Review-2 N3/L1: items trimmed, empty items dropped, an empty
            // list is unset (the env rule).
            (
                &["--only", "gitlab-ci, helm", "--skip", " helm ,"],
                Layer {
                    only: Some(vec!["gitlab-ci".into(), "helm".into()]),
                    skip: Some(vec!["helm".into()]),
                    ..Layer::default()
                },
            ),
            (&["--only", ",", "--skip", " "], Layer::default()),
            (
                &["--gitlab-host", "gitlab.example.com"],
                Layer {
                    gitlab_host: Some("gitlab.example.com".into()),
                    ..Layer::default()
                },
            ),
        ];
        for (args, want) in rows {
            assert_eq!(layer(&parsed(args).0), Ok(want), "{args:?}");
        }

        let (cli, _) = parsed(&[
            "--kind",
            "gitlab-ci:image=off",
            "--kind",
            "gitlab-ci:include=on",
        ]);
        let l = layer(&cli).unwrap();
        assert_eq!(
            l.plugins["gitlab-ci"].kinds,
            [("image".into(), false), ("include".into(), true)].into()
        );
        assert_eq!(l.plugins.len(), 1);

        let (cli, _) = parsed(&["--config", "x.toml", "--no-config"]);
        assert_eq!(cli.opts.config, Some(PathBuf::from("x.toml")));
        assert!(cli.opts.no_config);
        assert_eq!(
            layer(&cli),
            Ok(Layer::default()),
            "meta options are no fields"
        );

        let (cli, _) = parsed(&["--gitlab-host", "https://gitlab.example.com"]);
        assert_eq!(
            layer(&cli),
            Err(Fatal(
                "--gitlab-host: invalid GitLab host `https://gitlab.example.com`".into()
            ))
        );
    }

    #[test]
    fn paths_and_subcommands_parse() {
        let (cli, mode) = parsed(&["a.yml"]);
        assert_eq!((mode, paths(&cli)), (Mode::All, vec!["a.yml".into()]));

        let (cli, mode) = parsed(&["gitlab-ci", "a.yml"]);
        assert_eq!(
            (mode, paths(&cli)),
            (Mode::Plugin("gitlab-ci"), vec!["a.yml".into()])
        );

        let (cli, mode) = parsed(&["--check", "gitlab-ci", "--json", "a.yml"]);
        assert_eq!(
            (mode, paths(&cli)),
            (Mode::Plugin("gitlab-ci"), vec!["a.yml".into()])
        );
        assert!(
            cli.opts.check && cli.opts.json,
            "flags before and after the subcommand"
        );

        let (cli, mode) = parsed(&["--dry-run", "gitlab-ci"]);
        assert_eq!((mode, paths(&cli)), (Mode::Plugin("gitlab-ci"), vec![]));
        assert!(cli.opts.dry_run);

        let (cli, mode) = parsed(&["gitlab-ci", "a.yml", "--check"]);
        assert_eq!(
            (mode, paths(&cli)),
            (Mode::Plugin("gitlab-ci"), vec!["a.yml".into()])
        );
        assert!(cli.opts.check);

        let (cli, mode) = parsed(&["a.yml", "gitlab-ci", "b.yml"]);
        assert_eq!(
            (mode, paths(&cli)),
            (
                Mode::All,
                vec!["a.yml".into(), "gitlab-ci".into(), "b.yml".into()]
            ),
            "a subcommand only before the first path"
        );

        let (cli, mode) = parsed(&["plugins"]);
        assert_eq!((mode, paths(&cli)), (Mode::Plugins, vec![]));
        assert!(
            parse(&os(&["pin-toolkit", "plugins", "x"])).is_err(),
            "plugins takes no paths"
        );

        let (cli, mode) = parsed(&["./plugins"]);
        assert_eq!((mode, paths(&cli)), (Mode::All, vec!["./plugins".into()]));

        let (cli, mode) = parsed(&["helm"]);
        assert_eq!((mode, paths(&cli)), (Mode::Plugin("helm"), vec![]));
        let (_, mode) = parsed(&["github-actions"]);
        assert_eq!(mode, Mode::Plugin("github-actions"));

        let (cli, _) = parsed(&["--kind", "gitlab-ci:image=off"]);
        assert_eq!(
            layer(&cli).unwrap().plugins["gitlab-ci"].kinds,
            [("image".into(), false)].into()
        );
        for (v, want) in [
            (
                "gitlab-ci:image",
                "--kind expects <plugin>:<kind>=on|off, got \"gitlab-ci:image\"",
            ),
            (
                "gitlab-ci:image=yes",
                "--kind expects <plugin>:<kind>=on|off, got \"gitlab-ci:image=yes\"",
            ),
            (
                "image=on",
                "--kind expects <plugin>:<kind>=on|off, got \"image=on\"",
            ),
            ("helm:nope=on", "unknown kind `nope` for plugin `helm`"),
            (
                "gitlab:image=on",
                "unknown plugin `gitlab` (known: gitlab-ci, helm, github-actions)",
            ),
        ] {
            let (cli, _) = parsed(&["--kind", v]);
            assert_eq!(layer(&cli), Err(Fatal(want.into())), "--kind {v}");
        }
    }

    #[test]
    fn argv0_aliases() {
        let rest = ["--dry-run", "a.yml"];
        let with = |argv0: &str| {
            let mut a = os(&[argv0]);
            a.extend(os(&rest));
            dispatch_argv0(a)
        };
        for (argv0, inserted) in [
            ("pin-ci", Some("gitlab-ci")),
            ("/x/pin-gitlab-ci", Some("gitlab-ci")),
            ("pin-helm", Some("helm")),
            ("pin-github-actions", Some("github-actions")),
            ("pin-ci.exe", Some("gitlab-ci")),
            ("pin-toolkit", None),
            ("pin-foo", None),
            ("pin-ci.sh", None),
        ] {
            let mut want = os(&[argv0]);
            want.extend(inserted.map(OsString::from));
            want.extend(os(&rest));
            assert_eq!(with(argv0), want, "{argv0}");
        }
        assert_eq!(dispatch_argv0(vec![]), Vec::<OsString>::new());
    }
}
