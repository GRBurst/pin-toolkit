//! Configuration layers (spec §7.1, D35): every source (repo file, env, CLI;
//! the global file from M5e') produces the same [`Layer`], and
//! [`Effective::fold`] is the one place precedence is decided.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use pin_core::run::{AgePolicy, UpdatePolicy};

use crate::Process;
use crate::cli::{self, Cli};

use self::global::HostBinding;

pub mod env;
pub mod global;
pub mod repo;

/// The default `--concurrency`.
pub const DEFAULT_CONCURRENCY: usize = 8;

/// A plugin id list (`--only`/`--skip`, `PIN_TOOLKIT_ONLY`/`_SKIP`): items
/// trimmed, empty items dropped; nothing left = unset (T-D7, review-2 L1,
/// N3). Ids are checked where they are used (enablement).
pub fn id_list<'a>(items: impl IntoIterator<Item = &'a str>) -> Option<Vec<String>> {
    let ids: Vec<String> = items
        .into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// One configuration source. `None` (or an empty list/map) = not set here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub update: Option<bool>,
    pub update_major: Option<bool>,
    pub min_age_days: Option<u32>,
    pub unknown_age: Option<AgePolicy>,
    pub concurrency: Option<usize>,
    pub json: Option<bool>,
    pub dry_run: Option<bool>,
    pub check: Option<bool>,
    pub only: Option<Vec<String>>,
    pub skip: Option<Vec<String>>,
    /// The primary GitLab host (never from a repo file, §7.3).
    pub gitlab_host: Option<String>,
    /// `[hosts."<host>"]` bindings (global file only, §7.3); replaces
    /// lower layers' map.
    pub hosts: Option<BTreeMap<String, HostBinding>>,
    /// `[plugins.<id>]` by plugin id.
    pub plugins: BTreeMap<String, PluginLayer>,
}

/// One plugin's settings in one layer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginLayer {
    pub run: Option<bool>,
    /// Replaces the plugin's default globs (and lower layers' lists).
    pub files: Option<Vec<String>>,
    /// Concatenated across layers.
    pub ignore: Vec<String>,
    /// Per kind id; a higher layer wins per key.
    pub kinds: BTreeMap<String, bool>,
    /// gitlab-ci: hosts to resolve (replaces lower layers' lists).
    pub gitlab_hosts: Option<Vec<String>>,
    /// Plugin options without a toolkit field yet (helm `values`,
    /// `helmfile`, `aliases`): validated, unused until the plugin exists.
    pub options: Option<toml::Table>,
}

/// The folded configuration of one run.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    pub update: UpdatePolicy,
    pub concurrency: usize,
    pub json: bool,
    /// `dry_run ∨ check`.
    pub dry_run: bool,
    pub check: bool,
    pub only: Option<Vec<String>>,
    pub skip: Vec<String>,
    pub gitlab_host: Option<String>,
    pub hosts: BTreeMap<String, HostBinding>,
    pub plugins: BTreeMap<String, PluginLayer>,
}

impl Effective {
    /// Fold `layers`, lowest precedence first (T-D4, T-D5): scalars and the
    /// `files`/`gitlab-hosts`/`only`/`skip` lists and the `hosts` map take
    /// the last layer that sets them, `ignore` concatenates in layer order, `kinds` merge per
    /// key; unset fields get the built-in defaults. `check` implies
    /// `dry_run`.
    pub fn fold(layers: &[Layer]) -> Self {
        let mut acc = Layer::default();
        for l in layers {
            acc.merge(l);
        }
        let check = acc.check.unwrap_or(false);
        Self {
            update: UpdatePolicy {
                enabled: acc.update.unwrap_or(false),
                allow_major: acc.update_major.unwrap_or(false),
                min_age_days: acc.min_age_days.unwrap_or(0),
                unknown_age: acc.unknown_age.unwrap_or_default(),
            },
            concurrency: acc.concurrency.unwrap_or(DEFAULT_CONCURRENCY),
            json: acc.json.unwrap_or(false),
            dry_run: acc.dry_run.unwrap_or(false) || check,
            check,
            only: acc.only,
            skip: acc.skip.unwrap_or_default(),
            gitlab_host: acc.gitlab_host,
            hosts: acc.hosts.unwrap_or_default(),
            plugins: acc.plugins,
        }
    }
}

/// `*dst = src` when `src` is set.
fn over<T: Clone>(dst: &mut Option<T>, src: &Option<T>) {
    if src.is_some() {
        dst.clone_from(src);
    }
}

impl Layer {
    /// Apply the higher layer `h` on top of `self`.
    fn merge(&mut self, h: &Layer) {
        over(&mut self.update, &h.update);
        over(&mut self.update_major, &h.update_major);
        over(&mut self.min_age_days, &h.min_age_days);
        over(&mut self.unknown_age, &h.unknown_age);
        over(&mut self.concurrency, &h.concurrency);
        over(&mut self.json, &h.json);
        over(&mut self.dry_run, &h.dry_run);
        over(&mut self.check, &h.check);
        over(&mut self.only, &h.only);
        over(&mut self.skip, &h.skip);
        over(&mut self.gitlab_host, &h.gitlab_host);
        over(&mut self.hosts, &h.hosts);
        for (id, p) in &h.plugins {
            self.plugins.entry(id.clone()).or_default().merge(p);
        }
    }
}

impl PluginLayer {
    /// Apply the higher layer `h` on top of `self`.
    fn merge(&mut self, h: &PluginLayer) {
        over(&mut self.run, &h.run);
        over(&mut self.files, &h.files);
        self.ignore.extend(h.ignore.iter().cloned());
        self.kinds
            .extend(h.kinds.iter().map(|(k, on)| (k.clone(), *on)));
        over(&mut self.gitlab_hosts, &h.gitlab_hosts);
        over(&mut self.options, &h.options);
    }
}

/// The repo config file name at the git root.
pub const REPO_FILE: &str = ".pin-toolkit.toml";

/// The global config file: `<config dir>/pin-toolkit/config.toml`, where
/// the config dir is `$XDG_CONFIG_HOME` when it is non-empty and absolute
/// (the XDG rule: a relative value is invalid and ignored), else
/// `$HOME/.config` (the same on macOS, §7.1). `None` when neither gives a
/// directory. Pure: `env` is the process env map read at the edge.
pub fn global_path(env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let non_empty = |k: &str| env.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let dir = non_empty("XDG_CONFIG_HOME")
        .map(Path::new)
        .filter(|p| p.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| non_empty("HOME").map(|h| Path::new(h).join(".config")))?;
    Some(dir.join("pin-toolkit").join("config.toml"))
}

/// Where a configuration value comes from (`config show --origin`): the
/// layer order of [`Effective::fold`], lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Default,
    Global,
    Repo,
    Env,
    Cli,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Origin::Default => "default",
            Origin::Global => "global",
            Origin::Repo => "repo",
            Origin::Env => "env",
            Origin::Cli => "cli",
        })
    }
}

/// One run's configuration with its sources (`config show`, M5e').
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub eff: Effective,
    /// The global file that was read, as shown in messages (`None` = none).
    pub global_file: Option<String>,
    /// The repo file that was read, as shown in messages (`None` = none).
    pub repo_file: Option<String>,
    /// The layers that were folded, lowest precedence first (global and
    /// repo only when their file applies; env and CLI always).
    pub layers: Vec<(Origin, Layer)>,
    /// `GITLAB_URI` or `CI_SERVER_HOST` is set: where the primary GitLab
    /// host comes from when no layer sets `gitlab_host` (pin-core's
    /// `GitlabEnv::from_vars`, which reads them from the env map).
    pub env_primary: bool,
}

/// The highest layer in `layers` (lowest first) where `set` holds, or
/// [`Origin::Default`] when no layer sets the value.
pub fn origin_of(layers: &[(Origin, Layer)], set: impl Fn(&Layer) -> bool) -> Origin {
    layers
        .iter()
        .rev()
        .find(|(_, l)| set(l))
        .map_or(Origin::Default, |(o, _)| *o)
}

/// Every layer where `set` holds, highest first: the origins of a value
/// that concatenates across layers (`ignore`).
pub fn origins_of(layers: &[(Origin, Layer)], set: impl Fn(&Layer) -> bool) -> Vec<Origin> {
    layers
        .iter()
        .rev()
        .filter(|(_, l)| set(l))
        .map(|(o, _)| *o)
        .collect()
}

/// The effective configuration of one run: defaults < global file < repo
/// file < env < CLI. See [`load_sources`] for the file rules.
pub fn load(p: &Process, root: &Path, cli: &Cli) -> Result<Effective, Fatal> {
    load_sources(p, root, cli).map(|l| l.eff)
}

/// [`load`] with the layers it folded and the config files it read.
/// `--no-config` skips the global and the repo file.
/// Global file = [`global_path`] if it exists (a missing file is no layer).
/// Repo file = `--config` (must exist) : `PIN_TOOLKIT_CONFIG` (must exist)
/// : `<root>/.pin-toolkit.toml` if it exists; these override only the repo
/// file, the global file still applies. Relative paths are relative to
/// the cwd.
pub fn load_sources(p: &Process, root: &Path, cli: &Cli) -> Result<Loaded, Fatal> {
    let mut origins = Vec::with_capacity(4);
    let mut layers = Vec::with_capacity(4);
    let global = global_layer(p, cli)?;
    let global_file = global.as_ref().map(|(_, shown)| shown.clone());
    if let Some((layer, _)) = global {
        origins.push(Origin::Global);
        layers.push(layer);
    }
    let repo = repo_layer(p, root, cli)?;
    let repo_file = repo.as_ref().map(|(_, shown)| shown.clone());
    if let Some((layer, _)) = repo {
        origins.push(Origin::Repo);
        layers.push(layer);
    }
    origins.push(Origin::Env);
    layers.push(env::layer(&p.env)?);
    origins.push(Origin::Cli);
    layers.push(cli::layer(cli)?);
    let eff = Effective::fold(&layers);
    Ok(Loaded {
        eff,
        global_file,
        repo_file,
        layers: origins.into_iter().zip(layers).collect(),
        env_primary: ["GITLAB_URI", "CI_SERVER_HOST"]
            .iter()
            .any(|var| p.env.get(*var).is_some_and(|v| !v.is_empty())),
    })
}

/// The global layer and its file as shown in messages, if a global file
/// applies: not with `--no-config`, and only when [`global_path`] names a
/// file that exists. An unreadable file or one that does not parse is
/// fatal, named relative to the cwd.
fn global_layer(p: &Process, cli: &Cli) -> Result<Option<(Layer, String)>, Fatal> {
    if cli.opts.no_config {
        return Ok(None);
    }
    let Some(path) = global_path(&p.env).map(|g| p.cwd.join(g)) else {
        return Ok(None);
    };
    let display = PathBuf::from(crate::walk::shown(&p.cwd, &path));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Fatal(format!("{}: {e}", display.display()))),
    };
    let layer = global::parse(&display, &text)?;
    Ok(Some((layer, display.display().to_string())))
}

/// The repo layer and its file as shown in messages, if a repo file
/// applies.
fn repo_layer(p: &Process, root: &Path, cli: &Cli) -> Result<Option<(Layer, String)>, Fatal> {
    if cli.opts.no_config {
        return Ok(None);
    }
    let explicit = cli.opts.config.clone().or_else(|| env::config_path(&p.env));
    let (path, display) = match explicit {
        Some(given) => (p.cwd.join(&given), given),
        None => {
            let path = root.join(REPO_FILE);
            if !path.exists() {
                return Ok(None);
            }
            let display = PathBuf::from(crate::walk::shown(&p.cwd, &path));
            (path, display)
        }
    };
    let text = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Fatal(format!("config file not found: {}", display.display()))
        } else {
            Fatal(format!("{}: {e}", display.display()))
        }
    })?;
    let layer = repo::parse(&display, &text)?;
    Ok(Some((layer, display.display().to_string())))
}

/// A fatal error (exit 2 before the run, T-D22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fatal(pub String);

impl fmt::Display for Fatal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pin-toolkit: {}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// One field under test: set layer `k`'s value, read the folded value,
    /// and the expected value for "highest layer that set it" (`None` =
    /// the built-in default).
    struct Field {
        name: &'static str,
        set: fn(&mut Layer, usize),
        get: fn(&Effective) -> String,
        want: fn(Option<usize>) -> String,
    }

    fn b(k: usize) -> bool {
        k.is_multiple_of(2)
    }

    fn plugin(l: &mut Layer) -> &mut PluginLayer {
        l.plugins.entry("x".into()).or_default()
    }

    fn fields() -> Vec<Field> {
        vec![
            Field {
                name: "update",
                set: |l, k| l.update = Some(b(k)),
                get: |e| e.update.enabled.to_string(),
                want: |k| k.is_some_and(b).to_string(),
            },
            Field {
                name: "update_major",
                set: |l, k| l.update_major = Some(b(k)),
                get: |e| e.update.allow_major.to_string(),
                want: |k| k.is_some_and(b).to_string(),
            },
            Field {
                name: "min_age_days",
                set: |l, k| l.min_age_days = Some(10 + k as u32),
                get: |e| e.update.min_age_days.to_string(),
                want: |k| k.map_or(0, |k| 10 + k as u32).to_string(),
            },
            Field {
                name: "unknown_age",
                set: |l, k| {
                    l.unknown_age = Some(if b(k) {
                        AgePolicy::Block
                    } else {
                        AgePolicy::Allow
                    })
                },
                get: |e| format!("{:?}", e.update.unknown_age),
                want: |k| {
                    format!(
                        "{:?}",
                        if k.is_some_and(b) {
                            AgePolicy::Block
                        } else {
                            AgePolicy::Allow
                        }
                    )
                },
            },
            Field {
                name: "concurrency",
                set: |l, k| l.concurrency = Some(1 + k),
                get: |e| e.concurrency.to_string(),
                want: |k| k.map_or(DEFAULT_CONCURRENCY, |k| 1 + k).to_string(),
            },
            Field {
                name: "json",
                set: |l, k| l.json = Some(b(k)),
                get: |e| e.json.to_string(),
                want: |k| k.is_some_and(b).to_string(),
            },
            Field {
                name: "dry_run",
                set: |l, k| l.dry_run = Some(b(k)),
                get: |e| e.dry_run.to_string(),
                want: |k| k.is_some_and(b).to_string(),
            },
            Field {
                name: "check",
                set: |l, k| l.check = Some(b(k)),
                get: |e| e.check.to_string(),
                want: |k| k.is_some_and(b).to_string(),
            },
            Field {
                name: "only",
                set: |l, k| l.only = Some(vec![format!("p{k}")]),
                get: |e| format!("{:?}", e.only),
                want: |k| format!("{:?}", k.map(|k| vec![format!("p{k}")])),
            },
            Field {
                name: "skip",
                set: |l, k| l.skip = Some(vec![format!("p{k}")]),
                get: |e| format!("{:?}", e.skip),
                want: |k| format!("{:?}", k.map_or(vec![], |k| vec![format!("p{k}")])),
            },
            Field {
                name: "gitlab_host",
                set: |l, k| l.gitlab_host = Some(format!("h{k}.example")),
                get: |e| format!("{:?}", e.gitlab_host),
                want: |k| format!("{:?}", k.map(|k| format!("h{k}.example"))),
            },
            Field {
                name: "hosts",
                set: |l, k| {
                    l.hosts = Some(
                        [(
                            format!("h{k}.example"),
                            HostBinding::Gitlab {
                                token_env: format!("T{k}"),
                            },
                        )]
                        .into(),
                    )
                },
                get: |e| format!("{:?}", e.hosts),
                // Replace-merge: only the highest layer's map survives.
                want: |k| {
                    let map: BTreeMap<String, HostBinding> = k
                        .map(|k| {
                            (
                                format!("h{k}.example"),
                                HostBinding::Gitlab {
                                    token_env: format!("T{k}"),
                                },
                            )
                        })
                        .into_iter()
                        .collect();
                    format!("{map:?}")
                },
            },
            Field {
                name: "plugins.x.run",
                set: |l, k| plugin(l).run = Some(b(k)),
                get: |e| format!("{:?}", e.plugins.get("x").and_then(|p| p.run)),
                want: |k| format!("{:?}", k.map(b)),
            },
            Field {
                name: "plugins.x.kinds.image",
                set: |l, k| {
                    plugin(l).kinds.insert("image".into(), b(k));
                },
                get: |e| {
                    format!(
                        "{:?}",
                        e.plugins.get("x").map(|p| p.kinds.get("image").copied())
                    )
                },
                want: |k| format!("{:?}", k.map(|k| Some(b(k)))),
            },
            Field {
                name: "plugins.x.options",
                set: |l, k| {
                    let mut t = toml::Table::new();
                    t.insert("k".into(), toml::Value::Integer(k as i64));
                    plugin(l).options = Some(t);
                },
                get: |e| format!("{:?}", e.plugins.get("x").and_then(|p| p.options.clone())),
                want: |k| {
                    format!(
                        "{:?}",
                        k.map(|k| {
                            let mut t = toml::Table::new();
                            t.insert("k".into(), toml::Value::Integer(k as i64));
                            t
                        })
                    )
                },
            },
        ]
    }

    /// P-prec over every presence pattern of
    /// L = [defaults, global, repo, env, cli].
    #[test]
    fn fold_scalars_last_wins() {
        for f in fields() {
            for pattern in 0u8..32 {
                let mut layers = vec![Layer::default(); 5];
                for (k, l) in layers.iter_mut().enumerate() {
                    if pattern & (1 << k) != 0 {
                        (f.set)(l, k);
                    }
                }
                let highest = (0..5).rev().find(|k| pattern & (1 << k) != 0);
                assert_eq!(
                    (f.get)(&Effective::fold(&layers)),
                    (f.want)(highest),
                    "{} with layers {pattern:05b} set",
                    f.name
                );
            }
        }
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fold_lists_and_kinds() {
        let mut l0 = Layer::default();
        let p0 = plugin(&mut l0);
        p0.files = Some(strings(&["a", "b"]));
        p0.gitlab_hosts = Some(strings(&["g1", "g2"]));
        p0.ignore = strings(&["i0"]);
        p0.kinds = [("image".into(), false), ("include".into(), false)].into();
        let mut l1 = Layer::default();
        let p1 = plugin(&mut l1);
        p1.ignore = strings(&["i1a", "i1b"]);
        p1.kinds = [("image".into(), true)].into();
        let mut l2 = Layer::default();
        let p2 = plugin(&mut l2);
        p2.files = Some(strings(&["c"]));
        p2.gitlab_hosts = Some(strings(&["g3"]));
        let mut l3 = Layer::default();
        plugin(&mut l3).ignore = strings(&["i3"]);
        l3.check = Some(true);
        l3.dry_run = Some(false);

        let e = Effective::fold(&[l0, l1, l2, l3]);
        let x = &e.plugins["x"];
        assert_eq!(x.files, Some(strings(&["c"])), "files replace");
        assert_eq!(
            x.gitlab_hosts,
            Some(strings(&["g3"])),
            "gitlab-hosts replace"
        );
        assert_eq!(
            x.ignore,
            strings(&["i0", "i1a", "i1b", "i3"]),
            "ignore concatenates"
        );
        assert_eq!(
            x.kinds,
            [("image".into(), true), ("include".into(), false)].into(),
            "kinds per key"
        );
        assert!(e.check && e.dry_run, "check ⇒ dry_run");

        let d = Effective::fold(&[]);
        assert_eq!(d.concurrency, 8);
        assert_eq!(d.update, UpdatePolicy::default());
        assert!(!d.json && !d.dry_run && !d.check);
        assert_eq!((d.only, d.skip, d.gitlab_host), (None, vec![], None));
        assert!(d.hosts.is_empty() && d.plugins.is_empty());
    }

    fn env_of(vars: &[(&str, &str)]) -> BTreeMap<String, String> {
        vars.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn global_path_xdg_over_home() {
        let env = env_of(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/u")]);
        assert_eq!(
            global_path(&env),
            Some(PathBuf::from("/xdg/pin-toolkit/config.toml"))
        );
    }

    #[test]
    fn global_path_home_fallback() {
        let home = ("HOME", "/home/u");
        let want = Some(PathBuf::from("/home/u/.config/pin-toolkit/config.toml"));
        assert_eq!(global_path(&env_of(&[home])), want, "XDG unset");
        assert_eq!(
            global_path(&env_of(&[("XDG_CONFIG_HOME", ""), home])),
            want,
            "empty XDG ignored"
        );
        assert_eq!(
            global_path(&env_of(&[("XDG_CONFIG_HOME", "rel/xdg"), home])),
            want,
            "relative XDG ignored"
        );
    }

    #[test]
    fn global_path_none_without_a_config_dir() {
        assert_eq!(global_path(&env_of(&[])), None, "neither set");
        assert_eq!(
            global_path(&env_of(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "")])),
            None,
            "relative XDG and empty HOME"
        );
    }

    /// A fresh `$CARGO_MANIFEST_DIR/.tmp/<name>-<pid>/` with a `.git/`.
    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        dir
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// `load_sources` for `argv` in `root` (also the cwd) with `vars`; the
    /// global file is read from `<root>/xdg`.
    fn sources(root: &Path, argv: &[&str], vars: &[(&str, &str)]) -> Result<Loaded, Fatal> {
        let xdg = root.join("xdg").display().to_string();
        let mut env = env_of(vars);
        env.insert("XDG_CONFIG_HOME".into(), xdg);
        let mut args = vec![OsString::from("pin-toolkit")];
        args.extend(argv.iter().map(OsString::from));
        let (cli, _) = cli::parse(&args).unwrap();
        let p = Process {
            args,
            env,
            cwd: root.to_path_buf(),
        };
        load_sources(&p, root, &cli)
    }

    fn origins(l: &Loaded) -> Vec<Origin> {
        l.layers.iter().map(|(o, _)| *o).collect()
    }

    #[test]
    fn load_sources_origin_displays_lowercase() {
        let shown: Vec<String> = [
            Origin::Default,
            Origin::Global,
            Origin::Repo,
            Origin::Env,
            Origin::Cli,
        ]
        .iter()
        .map(Origin::to_string)
        .collect();
        assert_eq!(shown, ["default", "global", "repo", "env", "cli"]);
    }

    #[test]
    fn load_sources_without_files_has_env_and_cli_layers() {
        let root = scratch("sources-none");
        let l = sources(&root, &[], &[]).unwrap();
        assert_eq!((l.global_file.clone(), l.repo_file.clone()), (None, None));
        assert_eq!(origins(&l), [Origin::Env, Origin::Cli]);
        assert_eq!(
            l.eff,
            Effective::fold(&[Layer::default(), Layer::default()])
        );
    }

    #[test]
    fn load_sources_names_files_and_orders_layers() {
        let root = scratch("sources-all");
        write(
            &root,
            "xdg/pin-toolkit/config.toml",
            "version = 1\njson = true\n",
        );
        write(&root, REPO_FILE, "version = 1\n[update]\nenabled = true\n");
        let vars = [("PIN_TOOLKIT_CONCURRENCY", "3")];
        let l = sources(&root, &["--min-age-days", "7"], &vars).unwrap();
        assert_eq!(
            l.global_file.as_deref(),
            Some("xdg/pin-toolkit/config.toml")
        );
        assert_eq!(l.repo_file.as_deref(), Some(".pin-toolkit.toml"));
        assert_eq!(
            origins(&l),
            [Origin::Global, Origin::Repo, Origin::Env, Origin::Cli]
        );
        let set = |o: Origin| {
            let (_, layer) = l.layers.iter().find(|(x, _)| *x == o).unwrap();
            (
                layer.json,
                layer.update,
                layer.concurrency,
                layer.min_age_days,
            )
        };
        assert_eq!(set(Origin::Global), (Some(true), None, None, None));
        assert_eq!(set(Origin::Repo), (None, Some(true), None, None));
        assert_eq!(set(Origin::Env), (None, None, Some(3), None));
        assert_eq!(set(Origin::Cli), (None, None, None, Some(7)));
        assert!(l.eff.json && l.eff.update.enabled);
        assert_eq!((l.eff.concurrency, l.eff.update.min_age_days), (3, 7));
    }

    /// `load` is `load_sources(..).eff`.
    #[test]
    fn load_sources_agrees_with_load() {
        let root = scratch("sources-load");
        write(
            &root,
            "xdg/pin-toolkit/config.toml",
            "version = 1\nconcurrency = 2\n",
        );
        write(
            &root,
            REPO_FILE,
            "version = 1\n[release_age]\nmin_days = 4\n",
        );
        let xdg = root.join("xdg").display().to_string();
        let args = vec![OsString::from("pin-toolkit"), OsString::from("--json")];
        let (cli, _) = cli::parse(&args).unwrap();
        let p = Process {
            args,
            env: env_of(&[("XDG_CONFIG_HOME", xdg.as_str())]),
            cwd: root.clone(),
        };
        let loaded = load_sources(&p, &root, &cli).unwrap();
        assert_eq!(load(&p, &root, &cli), Ok(loaded.eff));
    }

    #[test]
    fn load_sources_no_config_and_explicit_repo_file() {
        let root = scratch("sources-flags");
        write(
            &root,
            "xdg/pin-toolkit/config.toml",
            "version = 1\njson = true\n",
        );
        write(&root, REPO_FILE, "version = 1\n");
        write(&root, "other.toml", "version = 1\n");

        let l = sources(&root, &["--no-config"], &[]).unwrap();
        assert_eq!((l.global_file.clone(), l.repo_file.clone()), (None, None));
        assert_eq!(origins(&l), [Origin::Env, Origin::Cli]);
        assert!(!l.eff.json, "the global file is skipped");

        // `--config` and PIN_TOOLKIT_CONFIG replace only the repo file, shown as given.
        let l = sources(&root, &["--config", "other.toml"], &[]).unwrap();
        assert_eq!(l.repo_file.as_deref(), Some("other.toml"));
        assert_eq!(
            l.global_file.as_deref(),
            Some("xdg/pin-toolkit/config.toml")
        );
        let l = sources(&root, &[], &[("PIN_TOOLKIT_CONFIG", "other.toml")]).unwrap();
        assert_eq!(l.repo_file.as_deref(), Some("other.toml"));
    }

    #[test]
    fn load_sources_bad_file_is_fatal() {
        let root = scratch("sources-bad");
        write(&root, "xdg/pin-toolkit/config.toml", "version = 2\n");
        assert_eq!(
            sources(&root, &[], &[]),
            Err(Fatal(
                "xdg/pin-toolkit/config.toml: unsupported version 2 (expected 1)".into()
            ))
        );
    }

    /// A layer with `json` set by `Layer`s in `pattern` (bit k = layer k).
    fn json_layers(pattern: u8) -> Vec<(Origin, Layer)> {
        [Origin::Global, Origin::Repo, Origin::Env, Origin::Cli]
            .into_iter()
            .enumerate()
            .map(|(k, o)| {
                let json = (pattern & (1 << k) != 0).then_some(true);
                (
                    o,
                    Layer {
                        json,
                        ..Layer::default()
                    },
                )
            })
            .collect()
    }

    /// The origin of a scalar is the highest layer that sets it, else
    /// `default`; checked over every presence pattern of the 4 layers.
    #[test]
    fn origin_of_is_the_highest_setting_layer() {
        let all = [Origin::Global, Origin::Repo, Origin::Env, Origin::Cli];
        for pattern in 0u8..16 {
            let layers = json_layers(pattern);
            let highest = (0..4).rev().find(|k| pattern & (1 << k) != 0);
            assert_eq!(
                origin_of(&layers, |l| l.json.is_some()),
                highest.map_or(Origin::Default, |k| all[k]),
                "layers {pattern:04b}"
            );
        }
    }

    /// The origins of a concatenating value: every setting layer, highest
    /// first; none = empty.
    #[test]
    fn origins_of_lists_setting_layers_highest_first() {
        let all = [Origin::Global, Origin::Repo, Origin::Env, Origin::Cli];
        for pattern in 0u8..16 {
            let want: Vec<Origin> = (0..4)
                .rev()
                .filter(|k| pattern & (1 << k) != 0)
                .map(|k| all[k])
                .collect();
            assert_eq!(
                origins_of(&json_layers(pattern), |l| l.json.is_some()),
                want,
                "layers {pattern:04b}"
            );
        }
        let ignoring = |layers: &[(Origin, Layer)]| {
            origins_of(layers, |l| {
                l.plugins.get("x").is_some_and(|p| !p.ignore.is_empty())
            })
        };
        let mut global = Layer::default();
        plugin(&mut global).ignore = strings(&["a"]);
        let mut repo = Layer::default();
        plugin(&mut repo).ignore = strings(&["b"]);
        let layers = [
            (Origin::Global, global),
            (Origin::Repo, repo),
            (Origin::Env, Layer::default()),
        ];
        assert_eq!(ignoring(&layers), [Origin::Repo, Origin::Global]);
        assert_eq!(ignoring(&layers[2..]), [], "no layer sets it");
    }

    /// The primary host's env fallback: `GITLAB_URI` or `CI_SERVER_HOST`,
    /// non-empty, in the env map.
    #[test]
    fn origin_env_primary_follows_the_host_env_vars() {
        let root = scratch("origin-env-primary");
        let primary = |vars: &[(&str, &str)]| sources(&root, &[], vars).unwrap().env_primary;
        assert!(!primary(&[]));
        assert!(
            !primary(&[("GITLAB_URI", ""), ("CI_SERVER_HOST", "")]),
            "empty is unset"
        );
        assert!(!primary(&[
            ("GITLAB_TOKEN", "x"),
            ("PIN_TOOLKIT_GITLAB_HOST", "h.example")
        ]));
        assert!(primary(&[("GITLAB_URI", "https://h.example/")]));
        assert!(primary(&[("CI_SERVER_HOST", "h.example")]));
    }
}
