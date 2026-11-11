//! `pin-toolkit config show` (spec §7.1, M5e'): the effective configuration
//! as dotted keys, one `key = value` per line, values TOML-ish (lists as
//! `["a", "b"]`). Pure over [`Loaded`]: no files, no env, no resolvers.
//! Host bindings print env-var NAMES only, never values (§7.3).

use pin_core::run::AgePolicy;

use crate::config::Loaded;
use crate::config::global::HostBinding;

/// `key = value` lines.
#[derive(Default)]
struct Lines(String);

impl Lines {
    fn push(&mut self, key: &str, value: impl std::fmt::Display) {
        self.0.push_str(&format!("{key} = {value}\n"));
    }
}

/// The `config show` text: `config.*`, `update.*`, `release-age.*`,
/// `concurrency`, `json`, `gitlab-host` (`primary`, the host the run would
/// use: it may come from `GITLAB_URI`/`CI_SERVER_HOST`), then `hosts.*`.
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
    hosts(&mut out, l);
    out.0
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
}
