//! `pin-toolkit config show` (spec §7.1, M5e'): the effective configuration
//! as dotted keys, one `key = value` per line, values TOML-ish (lists as
//! `["a", "b"]`). Pure over [`Loaded`]: no files, no env, no resolvers.
//! Host bindings print env-var NAMES only, never values (§7.3).
//!
//! With `--origin` each line ends `  # <origin>`: the highest layer that
//! sets the value (default, global, repo, env, cli; `default` when none
//! does), computed from [`Loaded::layers`]. `ignore` concatenates across
//! layers: its origin is the contributing layers in layer order (lowest
//! first, the order the patterns concatenate in), joined by `+`.

use std::fmt::Display;

use pin_core::plugin::DynPlugin;
use pin_core::run::AgePolicy;

use crate::config::global::HostBinding;
use crate::config::{Layer, Loaded, Origin, PluginLayer, origin_of, origins_of};
use crate::enable::{files, selection};
use crate::known::{KNOWN, compiled_plugin};

/// `key = value` lines, each with `  # <origin>` when `with_origin`.
struct Lines {
    text: String,
    with_origin: bool,
}

impl Lines {
    fn push(&mut self, key: &str, value: impl Display, origin: impl Display) {
        self.text.push_str(&format!("{key} = {value}"));
        if self.with_origin {
            self.text.push_str(&format!("  # {origin}"));
        }
        self.text.push('\n');
    }
}

/// A list as a TOML array of strings.
fn list(items: &[String]) -> String {
    let quoted: Vec<String> = items
        .iter()
        .map(|s| toml::Value::String(s.clone()).to_string())
        .collect();
    format!("[{}]", quoted.join(", "))
}

/// The layers a concatenated value comes from (`highest_first`, as
/// [`origins_of`] gives them) joined by `+` in layer order, the order the
/// items concatenate in: `global+repo` (`default` for none).
fn joined(highest_first: &[Origin]) -> String {
    if highest_first.is_empty() {
        return Origin::Default.to_string();
    }
    let names: Vec<String> = highest_first.iter().rev().map(Origin::to_string).collect();
    names.join("+")
}

/// The `config show` text: `config.*`, `update.*`, `release-age.*`,
/// `concurrency`, `json`, `gitlab-host` (`primary`, the host the run would
/// use: it may come from `GITLAB_URI`/`CI_SERVER_HOST`), `plugins.*`, then
/// `hosts.*`; with `origin`, each line ends with its origin.
pub fn render(l: &Loaded, primary: &str, origin: bool) -> String {
    let e = &l.eff;
    let mut out = Lines {
        text: String::new(),
        with_origin: origin,
    };
    let at = |set: fn(&Layer) -> bool| origin_of(&l.layers, set);
    // A file is shown for the layer it was read as; no file, no layer.
    let file_origin =
        |file: &Option<String>, layer: Origin| file.as_ref().map_or(Origin::Default, |_| layer);
    out.push(
        "config.global-file",
        l.global_file.as_deref().unwrap_or("none"),
        file_origin(&l.global_file, Origin::Global),
    );
    out.push(
        "config.repo-file",
        l.repo_file.as_deref().unwrap_or("none"),
        file_origin(&l.repo_file, Origin::Repo),
    );
    out.push(
        "update.enabled",
        e.update.enabled,
        at(|x| x.update.is_some()),
    );
    out.push(
        "update.major",
        e.update.allow_major,
        at(|x| x.update_major.is_some()),
    );
    out.push(
        "release-age.min-days",
        e.update.min_age_days,
        at(|x| x.min_age_days.is_some()),
    );
    out.push(
        "release-age.unknown",
        match e.update.unknown_age {
            AgePolicy::Allow => "allow",
            AgePolicy::Block => "block",
        },
        at(|x| x.unknown_age.is_some()),
    );
    out.push(
        "concurrency",
        e.concurrency,
        at(|x| x.concurrency.is_some()),
    );
    out.push("json", e.json, at(|x| x.json.is_some()));
    // No layer names the host: the env vars the run reads (`GITLAB_URI`,
    // `CI_SERVER_HOST`) or the built-in default.
    let host_origin = match at(|x| x.gitlab_host.is_some()) {
        Origin::Default if l.env_primary => Origin::Env,
        o => o,
    };
    out.push("gitlab-host", primary, host_origin);
    plugins(&mut out, l);
    hosts(&mut out, l);
    out.text
}

/// `plugins.<id>.*` in [`KNOWN`] order: every key of a plugin compiled in
/// (defaults filled as enablement computes them); of a plugin that is not,
/// only the keys its config table sets (nothing without a table).
fn plugins(out: &mut Lines, l: &Loaded) {
    for k in &KNOWN {
        let built = compiled_plugin(k.id);
        let table = l.eff.plugins.get(k.id);
        if built.is_some() || table.is_some() {
            plugin(
                out,
                l,
                k.id,
                built,
                table.unwrap_or(&PluginLayer::default()),
            );
        }
    }
}

fn plugin(
    out: &mut Lines,
    l: &Loaded,
    id: &str,
    built: Option<&'static dyn DynPlugin>,
    pl: &PluginLayer,
) {
    let key = |field: &str| format!("plugins.{id}.{field}");
    let set_in = |set: &dyn Fn(&PluginLayer) -> bool| {
        origin_of(&l.layers, |x| x.plugins.get(id).is_some_and(set))
    };
    if let Some(run) = pl.run.or(built.map(|_| true)) {
        out.push(&key("run"), run, set_in(&|x| x.run.is_some()));
    }
    if let Some(globs) = built.map(|p| files(&l.eff, p)).or_else(|| pl.files.clone()) {
        out.push(&key("files"), list(&globs), set_in(&|x| x.files.is_some()));
    }
    if built.is_some() || !pl.ignore.is_empty() {
        let layers = origins_of(&l.layers, |x| {
            x.plugins.get(id).is_some_and(|x| !x.ignore.is_empty())
        });
        out.push(&key("ignore"), list(&pl.ignore), joined(&layers));
    }
    // The compiled kinds (in the plugin's order), then the other kinds the
    // config sets (known, not compiled in yet).
    let mut kinds: Vec<(String, bool)> = built.map_or_else(Vec::new, |p| {
        let on = selection(&l.eff, p).kinds;
        p.kinds()
            .iter()
            .map(|k| (k.id.to_string(), on.contains(k.id)))
            .collect()
    });
    for (kind, on) in &pl.kinds {
        if !kinds.iter().any(|(k, _)| k == kind) {
            kinds.push((kind.clone(), *on));
        }
    }
    for (kind, on) in kinds {
        let origin = set_in(&|x| x.kinds.contains_key(&kind));
        out.push(&key(&format!("kinds.{kind}")), on, origin);
    }
    let hosts = pl
        .gitlab_hosts
        .clone()
        .or_else(|| (built.is_some() && id == "gitlab-ci").then(Vec::new));
    if let Some(hosts) = hosts {
        out.push(
            &key("gitlab-hosts"),
            list(&hosts),
            set_in(&|x| x.gitlab_hosts.is_some()),
        );
    }
    // The options table is replaced as a whole by the highest layer that has one.
    let options = set_in(&|x| x.options.is_some());
    for (name, value) in pl.options.iter().flatten() {
        option(out, &key(name), value, options);
    }
}

/// One plugin option (helm `values`, `helmfile`, `aliases`): a table as
/// dotted keys, any other value as TOML.
fn option(out: &mut Lines, key: &str, value: &toml::Value, origin: Origin) {
    match value {
        toml::Value::Table(t) if !t.is_empty() => {
            for (name, v) in t {
                option(out, &format!("{key}.{name}"), v, origin);
            }
        }
        toml::Value::Table(_) => out.push(key, "{}", origin),
        v => out.push(key, v, origin),
    }
}

/// `hosts.<host>.*`: the binding's kind and the env-var names it names. The
/// map is replaced as a whole by the highest layer that has one.
fn hosts(out: &mut Lines, l: &Loaded) {
    let origin = origin_of(&l.layers, |x| x.hosts.is_some());
    for (host, binding) in &l.eff.hosts {
        let key = |field: &str| format!("hosts.{host}.{field}");
        match binding {
            HostBinding::Gitlab { token_env } => {
                out.push(&key("kind"), "gitlab", origin);
                out.push(&key("token-env"), token_env, origin);
            }
            HostBinding::HelmRepo {
                username_env,
                password_env,
            } => {
                out.push(&key("kind"), "helm-repo", origin);
                out.push(&key("username-env"), username_env, origin);
                out.push(&key("password-env"), password_env, origin);
            }
            HostBinding::Oci { ca_file } => {
                out.push(&key("kind"), "oci", origin);
                out.push(&key("ca-file"), ca_file.display(), origin);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Effective, Layer, Origin};

    /// A `Loaded` over `layers` (lowest first) read from the given files.
    fn loaded(global: Option<&str>, repo: Option<&str>, layers: Vec<(Origin, Layer)>) -> Loaded {
        let plain: Vec<Layer> = layers.iter().map(|(_, l)| l.clone()).collect();
        Loaded {
            eff: Effective::fold(&plain),
            global_file: global.map(String::from),
            repo_file: repo.map(String::from),
            layers,
            env_primary: false,
        }
    }

    /// The lines of `text` that are not `plugins.*` (those are pinned by
    /// `plugins_*`, and depend on the plugins compiled in).
    fn core(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|l| !l.starts_with("plugins."))
            .collect()
    }

    #[test]
    fn renders_defaults() {
        let l = loaded(None, None, vec![(Origin::Env, Layer::default())]);
        assert_eq!(
            core(&render(&l, "gitlab.com", false)),
            [
                "config.global-file = none",
                "config.repo-file = none",
                "update.enabled = false",
                "update.major = false",
                "release-age.min-days = 0",
                "release-age.unknown = allow",
                "concurrency = 8",
                "json = false",
                "gitlab-host = gitlab.com",
            ]
        );
    }

    #[test]
    fn renders_set_values_and_files() {
        let global = Layer {
            json: Some(true),
            concurrency: Some(2),
            ..Layer::default()
        };
        let repo = Layer {
            update: Some(true),
            update_major: Some(true),
            min_age_days: Some(7),
            unknown_age: Some(AgePolicy::Block),
            ..Layer::default()
        };
        let l = loaded(
            Some("xdg/pin-toolkit/config.toml"),
            Some(".pin-toolkit.toml"),
            vec![(Origin::Global, global), (Origin::Repo, repo)],
        );
        assert_eq!(
            core(&render(&l, "gitlab.example.com:8443", false)),
            [
                "config.global-file = xdg/pin-toolkit/config.toml",
                "config.repo-file = .pin-toolkit.toml",
                "update.enabled = true",
                "update.major = true",
                "release-age.min-days = 7",
                "release-age.unknown = block",
                "concurrency = 2",
                "json = true",
                "gitlab-host = gitlab.example.com:8443",
            ]
        );
    }

    #[test]
    fn renders_hosts_with_env_var_names_only() {
        let global = Layer {
            hosts: Some(
                [
                    (
                        "gitlab.example.com".to_string(),
                        HostBinding::Gitlab {
                            token_env: "GITLAB_EXAMPLE_TOKEN".into(),
                        },
                    ),
                    (
                        "charts.example.com".to_string(),
                        HostBinding::HelmRepo {
                            username_env: "CHARTS_USER".into(),
                            password_env: "CHARTS_PASSWORD".into(),
                        },
                    ),
                    (
                        "registry.example.com".to_string(),
                        HostBinding::Oci {
                            ca_file: "/etc/ssl/corp-ca.pem".into(),
                        },
                    ),
                ]
                .into(),
            ),
            ..Layer::default()
        };
        let l = loaded(None, None, vec![(Origin::Global, global)]);
        let text = render(&l, "gitlab.com", false);
        let hosts: Vec<&str> = text.lines().filter(|l| l.starts_with("hosts.")).collect();
        assert_eq!(
            hosts,
            [
                "hosts.charts.example.com.kind = helm-repo",
                "hosts.charts.example.com.username-env = CHARTS_USER",
                "hosts.charts.example.com.password-env = CHARTS_PASSWORD",
                "hosts.gitlab.example.com.kind = gitlab",
                "hosts.gitlab.example.com.token-env = GITLAB_EXAMPLE_TOKEN",
                "hosts.registry.example.com.kind = oci",
                "hosts.registry.example.com.ca-file = /etc/ssl/corp-ca.pem",
            ]
        );
        assert!(text.ends_with('\n'), "every line ends with a newline");
    }

    /// The lines of `text` under `prefix`.
    fn under<'a>(text: &'a str, prefix: &str) -> Vec<&'a str> {
        text.lines().filter(|l| l.starts_with(prefix)).collect()
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn with_plugin(id: &str, pl: PluginLayer) -> Loaded {
        let layer = Layer {
            plugins: [(id.to_string(), pl)].into(),
            ..Layer::default()
        };
        loaded(None, None, vec![(Origin::Repo, layer)])
    }

    /// The defaults enablement computes: every compiled kind (all on), the
    /// default globs, no ignore patterns, no extra GitLab hosts.
    #[cfg(feature = "gitlab-ci")]
    #[test]
    fn plugins_gitlab_ci_defaults() {
        let l = loaded(None, None, vec![(Origin::Env, Layer::default())]);
        let text = render(&l, "gitlab.com", false);
        assert_eq!(
            under(&text, "plugins."),
            [
                "plugins.gitlab-ci.run = true",
                r#"plugins.gitlab-ci.files = [".gitlab-ci.yml", "**/*.gitlab-ci.yml", ".gitlab/ci/**/*.yml"]"#,
                "plugins.gitlab-ci.ignore = []",
                "plugins.gitlab-ci.kinds.image = true",
                "plugins.gitlab-ci.kinds.service = true",
                "plugins.gitlab-ci.kinds.component = true",
                "plugins.gitlab-ci.kinds.include = true",
                "plugins.gitlab-ci.gitlab-hosts = []",
            ]
        );
        let all: Vec<&str> = text.lines().collect();
        let first = all.iter().position(|l| l.starts_with("plugins.")).unwrap();
        assert_eq!(
            all[first - 1],
            "gitlab-host = gitlab.com",
            "after gitlab-host"
        );
    }

    #[cfg(feature = "gitlab-ci")]
    #[test]
    fn plugins_gitlab_ci_configured() {
        let global = PluginLayer {
            ignore: strings(&["a/*"]),
            ..PluginLayer::default()
        };
        let repo = PluginLayer {
            run: Some(false),
            files: Some(strings(&["ci/*.yml", "q\"x"])),
            ignore: strings(&["b/*"]),
            kinds: [
                ("image".to_string(), false),
                ("helm-oci".to_string(), false),
            ]
            .into(),
            gitlab_hosts: Some(strings(&["gitlab.example.com"])),
            ..PluginLayer::default()
        };
        let layer = |pl| Layer {
            plugins: [("gitlab-ci".to_string(), pl)].into(),
            ..Layer::default()
        };
        let l = loaded(
            None,
            None,
            vec![(Origin::Global, layer(global)), (Origin::Repo, layer(repo))],
        );
        assert_eq!(
            under(&render(&l, "gitlab.com", false), "plugins."),
            [
                "plugins.gitlab-ci.run = false",
                // A TOML literal string where a basic one would need escapes.
                r#"plugins.gitlab-ci.files = ["ci/*.yml", 'q"x']"#,
                r#"plugins.gitlab-ci.ignore = ["a/*", "b/*"]"#,
                "plugins.gitlab-ci.kinds.image = false",
                "plugins.gitlab-ci.kinds.service = true",
                "plugins.gitlab-ci.kinds.component = true",
                "plugins.gitlab-ci.kinds.include = true",
                // Set, known, not compiled in yet: listed after the compiled kinds.
                "plugins.gitlab-ci.kinds.helm-oci = false",
                r#"plugins.gitlab-ci.gitlab-hosts = ["gitlab.example.com"]"#,
            ]
        );
    }

    /// A plugin table for a known plugin that is not compiled in lists only
    /// the keys it sets; no table, no lines.
    #[test]
    fn plugins_unbuilt_lists_set_keys_only() {
        let options: toml::Table = toml::from_str(
            "values = { tag-digest = true }\nhelmfile = { oci-digest = false }\n\
             aliases = { stable = \"https://charts.helm.sh/stable\" }\nextra = [1, 2]",
        )
        .unwrap();
        let l = with_plugin(
            "helm",
            PluginLayer {
                run: Some(true),
                ignore: strings(&["x"]),
                kinds: [("values-image".to_string(), false)].into(),
                options: Some(options),
                ..PluginLayer::default()
            },
        );
        let text = render(&l, "gitlab.com", false);
        assert_eq!(
            under(&text, "plugins.helm."),
            [
                "plugins.helm.run = true",
                r#"plugins.helm.ignore = ["x"]"#,
                "plugins.helm.kinds.values-image = false",
                "plugins.helm.aliases.stable = \"https://charts.helm.sh/stable\"",
                "plugins.helm.extra = [1, 2]",
                "plugins.helm.helmfile.oci-digest = false",
                "plugins.helm.values.tag-digest = true",
            ]
        );
        assert!(
            under(&text, "plugins.github-actions.").is_empty(),
            "no table"
        );

        // An empty table sets nothing.
        let l = with_plugin("github-actions", PluginLayer::default());
        assert!(under(&render(&l, "gitlab.com", false), "plugins.github-actions.").is_empty());
        let l = with_plugin("helm", PluginLayer::default());
        assert!(under(&render(&l, "gitlab.com", false), "plugins.helm.").is_empty());
    }

    /// Without the gitlab-ci plugin (`--no-default-features`) its table is
    /// listed like any other unbuilt one.
    #[cfg(not(feature = "gitlab-ci"))]
    #[test]
    fn plugins_gitlab_ci_unbuilt() {
        let l = loaded(None, None, vec![(Origin::Env, Layer::default())]);
        assert!(under(&render(&l, "gitlab.com", false), "plugins.").is_empty());
        let l = with_plugin(
            "gitlab-ci",
            PluginLayer {
                gitlab_hosts: Some(strings(&["gitlab.example.com"])),
                ..PluginLayer::default()
            },
        );
        assert_eq!(
            under(&render(&l, "gitlab.com", false), "plugins."),
            [r#"plugins.gitlab-ci.gitlab-hosts = ["gitlab.example.com"]"#]
        );
    }

    /// `text` without the `  # <origin>` suffixes.
    fn plain(text: &str) -> String {
        text.lines()
            .map(|l| format!("{}\n", l.rsplit_once("  # ").map_or(l, |(a, _)| a)))
            .collect()
    }

    fn layer_of(f: impl FnOnce(&mut Layer)) -> Layer {
        let mut l = Layer::default();
        f(&mut l);
        l
    }

    fn ignoring(patterns: &[&str]) -> PluginLayer {
        PluginLayer {
            ignore: strings(patterns),
            ..PluginLayer::default()
        }
    }

    fn plugin_layer(id: &str, pl: PluginLayer) -> Layer {
        layer_of(|l| {
            l.plugins.insert(id.to_string(), pl);
        })
    }

    /// Nothing set anywhere: every line says `default`.
    #[test]
    fn origin_defaults() {
        let l = loaded(
            None,
            None,
            vec![
                (Origin::Env, Layer::default()),
                (Origin::Cli, Layer::default()),
            ],
        );
        let text = render(&l, "gitlab.com", true);
        assert_eq!(
            core(&text),
            [
                "config.global-file = none  # default",
                "config.repo-file = none  # default",
                "update.enabled = false  # default",
                "update.major = false  # default",
                "release-age.min-days = 0  # default",
                "release-age.unknown = allow  # default",
                "concurrency = 8  # default",
                "json = false  # default",
                "gitlab-host = gitlab.com  # default",
            ]
        );
        assert!(text.lines().all(|l| l.ends_with("  # default")), "{text}");
    }

    /// The suffix is the only difference from the plain output.
    #[test]
    fn origin_suffix_is_the_only_difference() {
        let layers = vec![
            (Origin::Global, layer_of(|l| l.json = Some(true))),
            (Origin::Repo, plugin_layer("helm", ignoring(&["x", "y"]))),
            (Origin::Cli, layer_of(|l| l.update = Some(true))),
        ];
        let l = loaded(Some("g.toml"), Some("r.toml"), layers);
        let with = render(&l, "gitlab.com", true);
        assert_ne!(with, render(&l, "gitlab.com", false));
        assert_eq!(plain(&with), render(&l, "gitlab.com", false));
    }

    /// A scalar's origin is the highest layer that sets it; a file's is the
    /// layer it was read as.
    #[test]
    fn origin_scalars_name_the_highest_layer() {
        let global = layer_of(|l| {
            l.json = Some(true);
            l.concurrency = Some(2);
            l.update = Some(false);
        });
        let repo = layer_of(|l| {
            l.update = Some(true);
            l.update_major = Some(true);
            l.min_age_days = Some(7);
        });
        let env = layer_of(|l| {
            l.concurrency = Some(5);
            l.unknown_age = Some(AgePolicy::Block);
        });
        let cli = layer_of(|l| l.min_age_days = Some(30));
        let l = loaded(
            Some("g.toml"),
            Some(".pin-toolkit.toml"),
            vec![
                (Origin::Global, global),
                (Origin::Repo, repo),
                (Origin::Env, env),
                (Origin::Cli, cli),
            ],
        );
        assert_eq!(
            core(&render(&l, "gitlab.com", true)),
            [
                "config.global-file = g.toml  # global",
                "config.repo-file = .pin-toolkit.toml  # repo",
                "update.enabled = true  # repo",
                "update.major = true  # repo",
                "release-age.min-days = 30  # cli",
                "release-age.unknown = block  # env",
                "concurrency = 5  # env",
                "json = true  # global",
                "gitlab-host = gitlab.com  # default",
            ]
        );
    }

    /// The primary: a layer's value (`--gitlab-host`: cli, the env layer's
    /// `PIN_TOOLKIT_GITLAB_HOST`: env), else `env` when `GITLAB_URI` or
    /// `CI_SERVER_HOST` is set, else `default`.
    #[test]
    fn origin_gitlab_host() {
        let host = |layers: Vec<(Origin, Layer)>, env_primary: bool| {
            let mut l = loaded(None, None, layers);
            l.env_primary = env_primary;
            render(&l, "h.example", true)
                .lines()
                .find(|l| l.starts_with("gitlab-host = "))
                .unwrap()
                .to_string()
        };
        let set = |o: Origin| (o, layer_of(|l| l.gitlab_host = Some("h.example".into())));
        let none = |o: Origin| (o, Layer::default());
        assert_eq!(
            host(vec![none(Origin::Env), none(Origin::Cli)], false),
            "gitlab-host = h.example  # default"
        );
        assert_eq!(
            host(vec![none(Origin::Env), none(Origin::Cli)], true),
            "gitlab-host = h.example  # env",
            "from GITLAB_URI / CI_SERVER_HOST"
        );
        assert_eq!(
            host(vec![set(Origin::Env), none(Origin::Cli)], false),
            "gitlab-host = h.example  # env",
            "from PIN_TOOLKIT_GITLAB_HOST"
        );
        assert_eq!(
            host(vec![none(Origin::Env), set(Origin::Cli)], true),
            "gitlab-host = h.example  # cli",
            "--gitlab-host beats the env fallback"
        );
        assert_eq!(
            host(vec![set(Origin::Global), none(Origin::Env)], true),
            "gitlab-host = h.example  # global",
            "a config layer beats the env fallback"
        );
    }

    /// `ignore` concatenates: every contributing layer, joined by `+` in
    /// layer order; the other plugin keys name the highest layer.
    #[test]
    fn origin_plugin_lists_and_kinds() {
        let global = plugin_layer("helm", ignoring(&["a"]));
        let repo = plugin_layer(
            "helm",
            PluginLayer {
                run: Some(false),
                ignore: strings(&["b"]),
                kinds: [("values-image".to_string(), false)].into(),
                ..PluginLayer::default()
            },
        );
        let cli = plugin_layer(
            "helm",
            PluginLayer {
                kinds: [("values-image".to_string(), true)].into(),
                ..PluginLayer::default()
            },
        );
        let l = loaded(
            None,
            None,
            vec![
                (Origin::Global, global),
                (Origin::Repo, repo),
                (Origin::Cli, cli),
            ],
        );
        assert_eq!(
            under(&render(&l, "gitlab.com", true), "plugins.helm."),
            [
                "plugins.helm.run = false  # repo",
                r#"plugins.helm.ignore = ["a", "b"]  # global+repo"#,
                "plugins.helm.kinds.values-image = true  # cli",
            ]
        );
        let only_repo = loaded(
            None,
            None,
            vec![(Origin::Repo, plugin_layer("helm", ignoring(&["b"])))],
        );
        assert!(
            render(&only_repo, "gitlab.com", true)
                .contains("plugins.helm.ignore = [\"b\"]  # repo\n")
        );
    }

    #[cfg(feature = "gitlab-ci")]
    #[test]
    fn origin_gitlab_ci_defaults_and_layers() {
        let repo = plugin_layer(
            "gitlab-ci",
            PluginLayer {
                files: Some(strings(&["ci/*.yml"])),
                ignore: strings(&["r/*"]),
                gitlab_hosts: Some(strings(&["gitlab.example.com"])),
                ..PluginLayer::default()
            },
        );
        let cli = plugin_layer(
            "gitlab-ci",
            PluginLayer {
                kinds: [("image".to_string(), false)].into(),
                ..PluginLayer::default()
            },
        );
        let l = loaded(None, None, vec![(Origin::Repo, repo), (Origin::Cli, cli)]);
        assert_eq!(
            under(&render(&l, "gitlab.com", true), "plugins."),
            [
                "plugins.gitlab-ci.run = true  # default",
                r#"plugins.gitlab-ci.files = ["ci/*.yml"]  # repo"#,
                r#"plugins.gitlab-ci.ignore = ["r/*"]  # repo"#,
                "plugins.gitlab-ci.kinds.image = false  # cli",
                "plugins.gitlab-ci.kinds.service = true  # default",
                "plugins.gitlab-ci.kinds.component = true  # default",
                "plugins.gitlab-ci.kinds.include = true  # default",
                r#"plugins.gitlab-ci.gitlab-hosts = ["gitlab.example.com"]  # repo"#,
            ]
        );
        let none = loaded(None, None, vec![(Origin::Env, Layer::default())]);
        assert!(
            render(&none, "gitlab.com", true)
                .contains("plugins.gitlab-ci.ignore = []  # default\n"),
            "no layer contributes"
        );
    }

    /// Plugin options and the `[hosts]` map are replaced as a whole by the
    /// highest layer that has one.
    #[test]
    fn origin_options_and_hosts() {
        let opts = |t: &str| Some(toml::from_str::<toml::Table>(t).unwrap());
        let repo = plugin_layer(
            "helm",
            PluginLayer {
                options: opts("aliases = { s = \"https://c.example\" }"),
                ..PluginLayer::default()
            },
        );
        let global = layer_of(|l| {
            l.hosts = Some(
                [(
                    "gitlab.example.com".to_string(),
                    HostBinding::Gitlab {
                        token_env: "T".into(),
                    },
                )]
                .into(),
            )
        });
        let l = loaded(
            None,
            None,
            vec![(Origin::Global, global), (Origin::Repo, repo)],
        );
        let text = render(&l, "gitlab.com", true);
        assert_eq!(
            under(&text, "plugins.helm."),
            [r#"plugins.helm.aliases.s = "https://c.example"  # repo"#]
        );
        assert_eq!(
            under(&text, "hosts."),
            [
                "hosts.gitlab.example.com.kind = gitlab  # global",
                "hosts.gitlab.example.com.token-env = T  # global",
            ]
        );
    }
}
