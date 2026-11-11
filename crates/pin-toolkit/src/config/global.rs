//! The global layer: `<config dir>/pin-toolkit/config.toml` (spec §7.1,
//! §7.2; M5e'). A `toml::Table` pre-pass gives the `version` messages,
//! then a typed parse with `deny_unknown_fields` rejects every other
//! unknown key (D17). Pure: the caller reads the file and names it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pin_core::run::valid_gitlab_host;
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::{Fatal, Layer};

/// One `[hosts."<host>"]` binding (spec §7.3), exactly one shape per
/// `kind`. Env-var NAMES (and a CA file path) only: secret values are
/// never read or stored here, the generated file may be world-readable.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "kebab-case",
    deny_unknown_fields
)]
pub enum HostBinding {
    /// A GitLab host: the env var holding its `PRIVATE-TOKEN`.
    Gitlab { token_env: String },
    /// A Helm repository: the env vars holding its basic-auth pair.
    HelmRepo {
        username_env: String,
        password_env: String,
    },
    /// An OCI registry: a PEM file of extra CA certificates.
    Oci { ca_file: PathBuf },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobalFile {
    #[serde(rename = "version")]
    _version: IgnoredAny,
    json: Option<bool>,
    concurrency: Option<usize>,
    hosts: Option<BTreeMap<String, HostBinding>>,
}

/// Parse the global config `text` read from `display` (shown in messages).
pub fn parse(display: &Path, text: &str) -> Result<Layer, Fatal> {
    let p = display.display();
    let fatal = |msg: String| Fatal(format!("{p}: {msg}"));
    let table: toml::Table =
        toml::from_str(text).map_err(|e| fatal(e.to_string().trim_end().to_string()))?;
    match table.get("version") {
        None => return Err(fatal("missing `version = 1`".into())),
        Some(toml::Value::Integer(1)) => {}
        Some(v) => return Err(fatal(format!("unsupported version {v} (expected 1)"))),
    }
    let file: GlobalFile =
        toml::from_str(text).map_err(|e| fatal(e.to_string().trim_end().to_string()))?;
    if let Some(h) = file
        .hosts
        .iter()
        .flat_map(BTreeMap::keys)
        .find(|h| !valid_gitlab_host(h))
    {
        return Err(fatal(format!("invalid GitLab host `{h}` in hosts")));
    }
    Ok(Layer {
        json: file.json,
        concurrency: file.concurrency,
        hosts: file.hosts,
        ..Layer::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Effective;

    const PATH: &str = "home/.config/pin-toolkit/config.toml";

    fn parse_text(text: &str) -> Result<Layer, Fatal> {
        parse(Path::new(PATH), text)
    }

    #[test]
    fn global_json_and_concurrency() {
        let layer = parse_text("version = 1\njson = true\nconcurrency = 4\n").unwrap();
        assert_eq!(
            layer,
            Layer {
                json: Some(true),
                concurrency: Some(4),
                ..Layer::default()
            }
        );
        assert_eq!(parse_text("version = 1\n"), Ok(Layer::default()));
    }

    #[test]
    fn global_rejects() {
        let rows = [
            ("json = true\n", "missing `version = 1`"),
            ("version = 2\n", "unsupported version 2 (expected 1)"),
            (
                "version = \"1\"\n",
                "unsupported version \"1\" (expected 1)",
            ),
            ("version = 1\njsno = true\n", "unknown field `jsno`"),
            (
                "version = 1\n[update]\nenabled = true\n",
                "unknown field `update`",
            ),
            ("version = 1\nconcurrency = -1\n", "expected usize"),
            ("version = 1\njson = \"yes\"\n", "invalid type"),
            ("version = [\n", "expected"),
        ];
        for (text, want) in rows {
            let got = parse_text(text).expect_err(text).0;
            assert!(got.starts_with(&format!("{PATH}: ")), "{text}: {got}");
            assert!(got.contains(want), "{text}: {got}");
        }
    }

    const HOSTS: &str = r#"version = 1
[hosts."gitlab.example.com"]
kind = "gitlab"
token-env = "GITLAB_EXAMPLE_TOKEN"
[hosts."charts.example.com:8443"]
kind = "helm-repo"
username-env = "CHARTS_USER"
password-env = "CHARTS_PASSWORD"
[hosts."registry.example.com"]
kind = "oci"
ca-file = "/etc/ssl/certs/corp-ca.pem"
"#;

    fn binding(kind: &str, fields: &str) -> String {
        format!("version = 1\n[hosts.\"h.example\"]\nkind = \"{kind}\"\n{fields}")
    }

    #[test]
    fn hosts_each_kind_ok() {
        let want: BTreeMap<String, HostBinding> = [
            (
                "gitlab.example.com".to_string(),
                HostBinding::Gitlab {
                    token_env: "GITLAB_EXAMPLE_TOKEN".into(),
                },
            ),
            (
                "charts.example.com:8443".to_string(),
                HostBinding::HelmRepo {
                    username_env: "CHARTS_USER".into(),
                    password_env: "CHARTS_PASSWORD".into(),
                },
            ),
            (
                "registry.example.com".to_string(),
                HostBinding::Oci {
                    ca_file: "/etc/ssl/certs/corp-ca.pem".into(),
                },
            ),
        ]
        .into();
        assert_eq!(
            parse_text(HOSTS),
            Ok(Layer {
                hosts: Some(want),
                ..Layer::default()
            })
        );
        assert_eq!(parse_text("version = 1\n").unwrap().hosts, None);
    }

    #[test]
    fn hosts_bad_kind() {
        for text in [
            binding("gitlab-ci", ""),
            binding("GitLab", "token-env = \"T\"\n"),
            "version = 1\n[hosts.\"h.example\"]\ntoken-env = \"T\"\n".to_string(),
        ] {
            let got = parse_text(&text).expect_err(&text).0;
            assert!(got.starts_with(&format!("{PATH}: ")), "{text}: {got}");
            assert!(
                got.contains("unknown variant") || got.contains("missing field `kind`"),
                "{text}: {got}"
            );
        }
    }

    #[test]
    fn hosts_missing_field() {
        let rows = [
            ("gitlab", "", "token-env"),
            ("helm-repo", "username-env = \"U\"\n", "password-env"),
            ("helm-repo", "password-env = \"P\"\n", "username-env"),
            ("oci", "", "ca-file"),
        ];
        for (kind, fields, missing) in rows {
            let text = binding(kind, fields);
            let got = parse_text(&text).expect_err(&text).0;
            assert!(got.starts_with(&format!("{PATH}: ")), "{text}: {got}");
            assert!(
                got.contains(&format!("missing field `{missing}`")),
                "{text}: {got}"
            );
        }
    }

    #[test]
    fn hosts_extra_field() {
        let rows = [
            (
                "gitlab",
                "token-env = \"T\"\nusername-env = \"U\"\n",
                "username-env",
            ),
            (
                "gitlab",
                "token-env = \"T\"\nca-file = \"/ca.pem\"\n",
                "ca-file",
            ),
            ("gitlab", "token-env = \"T\"\ntoken = \"secret\"\n", "token"),
            (
                "helm-repo",
                "username-env = \"U\"\npassword-env = \"P\"\ntoken-env = \"T\"\n",
                "token-env",
            ),
            (
                "oci",
                "ca-file = \"/ca.pem\"\ntoken-env = \"T\"\n",
                "token-env",
            ),
        ];
        for (kind, fields, extra) in rows {
            let text = binding(kind, fields);
            let got = parse_text(&text).expect_err(&text).0;
            assert!(got.starts_with(&format!("{PATH}: ")), "{text}: {got}");
            assert!(
                got.contains(&format!("unknown field `{extra}`")),
                "{text}: {got}"
            );
        }
    }

    #[test]
    fn hosts_bad_key() {
        for key in ["https://x", "", "a b", "a@b", "h:x", "-h.example"] {
            let text =
                format!("version = 1\n[hosts.\"{key}\"]\nkind = \"gitlab\"\ntoken-env = \"T\"\n");
            assert_eq!(
                parse_text(&text),
                Err(Fatal(format!(
                    "{PATH}: invalid GitLab host `{key}` in hosts"
                ))),
                "{key}"
            );
        }
    }

    #[test]
    fn hosts_layer_merge_replace() {
        let layer = |text: &str| parse_text(text).unwrap();
        let low = layer(HOSTS);
        let high = layer(&binding("gitlab", "token-env = \"T\"\n"));
        let neither = layer("version = 1\njson = true\n");

        let replaced = Effective::fold(&[low.clone(), high]);
        assert_eq!(replaced.hosts.keys().collect::<Vec<_>>(), ["h.example"]);
        let kept = Effective::fold(&[low, neither]);
        assert_eq!(kept.hosts.len(), 3, "an unset layer keeps lower hosts");
        assert!(Effective::fold(&[]).hosts.is_empty());
    }
}
