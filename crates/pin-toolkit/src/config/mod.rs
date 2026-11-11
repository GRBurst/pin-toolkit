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

/// The effective configuration of one run: defaults < repo file < env <
/// CLI. Repo file = `--no-config` ? none : `--config` (must exist) :
/// `PIN_TOOLKIT_CONFIG` (must exist) : `<root>/.pin-toolkit.toml` if it
/// exists. Relative paths are relative to the cwd.
pub fn load(p: &Process, root: &Path, cli: &Cli) -> Result<Effective, Fatal> {
    let mut layers = Vec::with_capacity(3);
    if let Some(repo) = repo_layer(p, root, cli)? {
        layers.push(repo);
    }
    layers.push(env::layer(&p.env)?);
    layers.push(cli::layer(cli)?);
    Ok(Effective::fold(&layers))
}

/// The repo layer, if a repo file applies.
fn repo_layer(p: &Process, root: &Path, cli: &Cli) -> Result<Option<Layer>, Fatal> {
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
    repo::parse(&display, &text).map(Some)
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

    /// P-prec over every presence pattern of L = [defaults, repo, env, cli].
    #[test]
    fn fold_scalars_last_wins() {
        for f in fields() {
            for pattern in 0u8..16 {
                let mut layers = vec![Layer::default(); 4];
                for (k, l) in layers.iter_mut().enumerate() {
                    if pattern & (1 << k) != 0 {
                        (f.set)(l, k);
                    }
                }
                let highest = (0..4).rev().find(|k| pattern & (1 << k) != 0);
                assert_eq!(
                    (f.get)(&Effective::fold(&layers)),
                    (f.want)(highest),
                    "{} with layers {pattern:04b} set",
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
}
