//! `pin-toolkit config show` (spec §7.1, M5e'): the effective configuration
//! as dotted keys, one `key = value` per line, values TOML-ish (lists as
//! `["a", "b"]`). Pure over [`Loaded`]: no files, no env, no resolvers.
//! Host bindings print env-var NAMES only, never values (§7.3).

use pin_core::run::AgePolicy;

use pin_core::plugin::DynPlugin;

use crate::config::global::HostBinding;
use crate::config::{Loaded, PluginLayer};
use crate::enable::{files, selection};
use crate::known::{KNOWN, compiled_plugin};

/// `key = value` lines.
#[derive(Default)]
struct Lines(String);

impl Lines {
    fn push(&mut self, key: &str, value: impl std::fmt::Display) {
        self.0.push_str(&format!("{key} = {value}\n"));
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

/// The `config show` text: `config.*`, `update.*`, `release-age.*`,
/// `concurrency`, `json`, `gitlab-host` (`primary`, the host the run would
/// use: it may come from `GITLAB_URI`/`CI_SERVER_HOST`), `plugins.*`, then
/// `hosts.*`.
pub fn render(l: &Loaded, primary: &str) -> String {
    let e = &l.eff;
    let mut out = Lines::default();
    out.push(
        "config.global-file",
        l.global_file.as_deref().unwrap_or("none"),
    );
    out.push("config.repo-file", l.repo_file.as_deref().unwrap_or("none"));
    out.push("update.enabled", e.update.enabled);
    out.push("update.major", e.update.allow_major);
    out.push("release-age.min-days", e.update.min_age_days);
    out.push(
        "release-age.unknown",
        match e.update.unknown_age {
            AgePolicy::Allow => "allow",
            AgePolicy::Block => "block",
        },
    );
    out.push("concurrency", e.concurrency);
    out.push("json", e.json);
    out.push("gitlab-host", primary);
    plugins(&mut out, l);
    hosts(&mut out, l);
    out.0
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
    if let Some(run) = pl.run.or(built.map(|_| true)) {
        out.push(&key("run"), run);
    }
    if let Some(globs) = built.map(|p| files(&l.eff, p)).or_else(|| pl.files.clone()) {
        out.push(&key("files"), list(&globs));
    }
    if built.is_some() || !pl.ignore.is_empty() {
        out.push(&key("ignore"), list(&pl.ignore));
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
        out.push(&key(&format!("kinds.{kind}")), on);
    }
    let hosts = pl
        .gitlab_hosts
        .clone()
        .or_else(|| (built.is_some() && id == "gitlab-ci").then(Vec::new));
    if let Some(hosts) = hosts {
        out.push(&key("gitlab-hosts"), list(&hosts));
    }
    for (name, value) in pl.options.iter().flatten() {
        option(out, &key(name), value);
    }
}

/// One plugin option (helm `values`, `helmfile`, `aliases`): a table as
/// dotted keys, any other value as TOML.
fn option(out: &mut Lines, key: &str, value: &toml::Value) {
    match value {
        toml::Value::Table(t) if !t.is_empty() => {
            for (name, v) in t {
                option(out, &format!("{key}.{name}"), v);
            }
        }
        toml::Value::Table(_) => out.push(key, "{}"),
        v => out.push(key, v),
    }
}

/// `hosts.<host>.*`: the binding's kind and the env-var names it names.
fn hosts(out: &mut Lines, l: &Loaded) {
    for (host, binding) in &l.eff.hosts {
        let key = |field: &str| format!("hosts.{host}.{field}");
        match binding {
            HostBinding::Gitlab { token_env } => {
                out.push(&key("kind"), "gitlab");
                out.push(&key("token-env"), token_env);
            }
            HostBinding::HelmRepo {
                username_env,
                password_env,
            } => {
                out.push(&key("kind"), "helm-repo");
                out.push(&key("username-env"), username_env);
                out.push(&key("password-env"), password_env);
            }
            HostBinding::Oci { ca_file } => {
                out.push(&key("kind"), "oci");
                out.push(&key("ca-file"), ca_file.display());
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
            core(&render(&l, "gitlab.com")),
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
            core(&render(&l, "gitlab.example.com:8443")),
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
        let text = render(&l, "gitlab.com");
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
        let text = render(&l, "gitlab.com");
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
            under(&render(&l, "gitlab.com"), "plugins."),
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
        let text = render(&l, "gitlab.com");
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
        assert!(under(&render(&l, "gitlab.com"), "plugins.github-actions.").is_empty());
        let l = with_plugin("helm", PluginLayer::default());
        assert!(under(&render(&l, "gitlab.com"), "plugins.helm.").is_empty());
    }

    /// Without the gitlab-ci plugin (`--no-default-features`) its table is
    /// listed like any other unbuilt one.
    #[cfg(not(feature = "gitlab-ci"))]
    #[test]
    fn plugins_gitlab_ci_unbuilt() {
        let l = loaded(None, None, vec![(Origin::Env, Layer::default())]);
        assert!(under(&render(&l, "gitlab.com"), "plugins.").is_empty());
        let l = with_plugin(
            "gitlab-ci",
            PluginLayer {
                gitlab_hosts: Some(strings(&["gitlab.example.com"])),
                ..PluginLayer::default()
            },
        );
        assert_eq!(
            under(&render(&l, "gitlab.com"), "plugins."),
            [r#"plugins.gitlab-ci.gitlab-hosts = ["gitlab.example.com"]"#]
        );
    }
}
