//! The repo layer: `.pin-toolkit.toml` (spec §7.1, §7.2; T-D6). A
//! `toml::Table` pre-pass gives the specific messages (global-only keys,
//! `version`, unknown plugin ids), then a typed parse with
//! `deny_unknown_fields` rejects every other unknown key (D17).

use std::collections::BTreeMap;
use std::path::Path;

use pin_core::run::{AgePolicy, valid_gitlab_host};
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::{Fatal, Layer, PluginLayer};
use crate::enable::file_glob;
use crate::known::{KNOWN, known};

/// Keys only the global config may set (§7.2): exit 2 in a repo file.
pub const GLOBAL_ONLY: [&str; 2] = ["hosts", "concurrency"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepoFile {
    #[serde(rename = "version")]
    _version: IgnoredAny,
    update: Option<UpdateTable>,
    release_age: Option<ReleaseAgeTable>,
    plugins: Option<PluginsTable>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateTable {
    enabled: Option<bool>,
    major: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseAgeTable {
    min_days: Option<u32>,
    unknown: Option<AgeValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AgeValue {
    Allow,
    Block,
}

/// One field per [`KNOWN`] id (unknown ids are rejected by the pre-pass).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginsTable {
    #[serde(rename = "gitlab-ci")]
    gitlab_ci: Option<GitlabCiTable>,
    helm: Option<HelmTable>,
    #[serde(rename = "github-actions")]
    github_actions: Option<GithubActionsTable>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct GitlabCiTable {
    run: Option<bool>,
    files: Option<Vec<String>>,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    kinds: BTreeMap<String, bool>,
    gitlab_hosts: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct HelmTable {
    run: Option<bool>,
    files: Option<Vec<String>>,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    kinds: BTreeMap<String, bool>,
    #[serde(rename = "values")]
    _values: Option<HelmValues>,
    #[serde(rename = "helmfile")]
    _helmfile: Option<HelmHelmfile>,
    aliases: Option<BTreeMap<String, String>>,
}

/// Validated only: helm options are unused until M8.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct HelmValues {
    #[serde(rename = "tag-digest")]
    _tag_digest: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct HelmHelmfile {
    #[serde(rename = "oci-digest")]
    _oci_digest: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct GithubActionsTable {
    run: Option<bool>,
    files: Option<Vec<String>>,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    kinds: BTreeMap<String, bool>,
}

/// The helm keys kept in [`PluginLayer::options`].
const HELM_OPTIONS: [&str; 3] = ["values", "helmfile", "aliases"];

/// Parse the repo config `text` read from `path` (shown in messages).
pub fn parse(path: &Path, text: &str) -> Result<Layer, Fatal> {
    let p = path.display();
    let fatal = |msg: String| Fatal(format!("{p}: {msg}"));
    let table: toml::Table =
        toml::from_str(text).map_err(|e| fatal(e.to_string().trim_end().to_string()))?;
    for key in GLOBAL_ONLY {
        if table.contains_key(key) {
            return Err(fatal(format!(
                "`{key}` is a global-only key; it is not allowed in a repository config"
            )));
        }
    }
    match table.get("version") {
        None => return Err(fatal("missing `version = 1`".into())),
        Some(toml::Value::Integer(1)) => {}
        Some(v) => return Err(fatal(format!("unsupported version {v} (expected 1)"))),
    }
    if let Some(plugins) = table.get("plugins").and_then(toml::Value::as_table) {
        for id in plugins.keys() {
            if known(id).is_none() {
                return Err(fatal(unknown_plugin(id)));
            }
        }
    }
    let file: RepoFile =
        toml::from_str(text).map_err(|e| fatal(e.to_string().trim_end().to_string()))?;

    let mut layer = Layer::default();
    if let Some(u) = file.update {
        layer.update = u.enabled;
        layer.update_major = u.major;
    }
    if let Some(r) = file.release_age {
        layer.min_age_days = r.min_days;
        layer.unknown_age = r.unknown.map(|u| match u {
            AgeValue::Allow => AgePolicy::Allow,
            AgeValue::Block => AgePolicy::Block,
        });
    }
    let Some(plugins) = file.plugins else {
        return Ok(layer);
    };
    if let Some(t) = plugins.gitlab_ci {
        if let Some(h) = t
            .gitlab_hosts
            .iter()
            .flatten()
            .find(|h| !valid_gitlab_host(h))
        {
            return Err(fatal(format!(
                "invalid GitLab host `{h}` in plugins.gitlab-ci.gitlab-hosts"
            )));
        }
        let pl = PluginLayer {
            run: t.run,
            files: t.files,
            ignore: t.ignore,
            kinds: t.kinds,
            gitlab_hosts: t.gitlab_hosts,
            options: None,
        };
        layer.plugins.insert("gitlab-ci".into(), pl);
    }
    if let Some(t) = plugins.helm {
        for (name, url) in t.aliases.iter().flatten() {
            if has_userinfo(url) {
                return Err(fatal(format!(
                    "plugins.helm.aliases.{name}: a repository URL must not contain credentials"
                )));
            }
        }
        let raw = table["plugins"]["helm"]
            .as_table()
            .cloned()
            .unwrap_or_default();
        let options: toml::Table = raw
            .into_iter()
            .filter(|(k, _)| HELM_OPTIONS.contains(&k.as_str()))
            .collect();
        let pl = PluginLayer {
            run: t.run,
            files: t.files,
            ignore: t.ignore,
            kinds: t.kinds,
            gitlab_hosts: None,
            options: (!options.is_empty()).then_some(options),
        };
        layer.plugins.insert("helm".into(), pl);
    }
    if let Some(t) = plugins.github_actions {
        let pl = PluginLayer {
            run: t.run,
            files: t.files,
            ignore: t.ignore,
            kinds: t.kinds,
            gitlab_hosts: None,
            options: None,
        };
        layer.plugins.insert("github-actions".into(), pl);
    }
    for (id, pl) in &layer.plugins {
        let k = known(id).expect("plugin ids checked above");
        if let Some(kind) = pl
            .kinds
            .keys()
            .find(|kind| !k.kinds.contains(&kind.as_str()))
        {
            return Err(fatal(unknown_kind(id, kind)));
        }
        for g in pl.files.iter().flatten() {
            file_glob(id, g).map_err(fatal)?;
        }
    }
    Ok(layer)
}

/// `unknown plugin `{id}` (known: …)`.
pub fn unknown_plugin(id: &str) -> String {
    let ids: Vec<&str> = KNOWN.iter().map(|k| k.id).collect();
    format!("unknown plugin `{id}` (known: {})", ids.join(", "))
}

/// `unknown kind `{kind}` for plugin `{id}``.
pub fn unknown_kind(id: &str, kind: &str) -> String {
    format!("unknown kind `{kind}` for plugin `{id}`")
}

/// The URL's authority (after `://`, else from the start, up to the first
/// `/`, `?` or `#`) holds userinfo (`user[:pass]@host`).
fn has_userinfo(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority.contains('@')
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "repo/.pin-toolkit.toml";

    /// The schema from the plan's Snippets, verbatim.
    const FULL: &str = r#"version = 1                                   # required, must be 1
[update]
enabled = false                               # bool
major = false                                 # bool (meaningful only with enabled)
[release_age]
min_days = 0                                  # u32
unknown = "allow"                             # "allow" | "block"
[plugins.gitlab-ci]                           # ids ∈ KNOWN; tables for unbuilt known plugins are validated too
run = true                                    # bool
files = [".gitlab-ci.yml", "**/*.gitlab-ci.yml", ".gitlab/ci/**/*.yml"]   # replaces defaults
ignore = ["registry.example.com/legacy/*"]    # concatenates across layers
gitlab-hosts = ["gitlab.com", "gitlab.example.com"]   # hosts to resolve; never credentials
kinds = { image = true, service = true, component = true, include = true, helm-oci = true }
[plugins.helm]                                # accepted (O1), unused until M7
run = true
kinds = { chart-dependency = true, values-image = true, helmfile-release = true }
values = { tag-digest = true }
helmfile = { oci-digest = false }
aliases = { stable = "https://charts.helm.sh/stable" }
[plugins.github-actions]
run = true
kinds = { action = true, docker-action = true, container = true }
"#;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn kinds(v: &[&str]) -> BTreeMap<String, bool> {
        v.iter().map(|k| (k.to_string(), true)).collect()
    }

    fn parse_text(text: &str) -> Result<Layer, Fatal> {
        parse(Path::new(PATH), text)
    }

    #[test]
    fn repo_full_schema_parses() {
        let mut want = Layer {
            update: Some(false),
            update_major: Some(false),
            min_age_days: Some(0),
            unknown_age: Some(AgePolicy::Allow),
            ..Layer::default()
        };
        want.plugins.insert(
            "gitlab-ci".into(),
            PluginLayer {
                run: Some(true),
                files: Some(strings(&[
                    ".gitlab-ci.yml",
                    "**/*.gitlab-ci.yml",
                    ".gitlab/ci/**/*.yml",
                ])),
                ignore: strings(&["registry.example.com/legacy/*"]),
                kinds: kinds(&["image", "service", "component", "include", "helm-oci"]),
                gitlab_hosts: Some(strings(&["gitlab.com", "gitlab.example.com"])),
                options: None,
            },
        );
        let options: toml::Table = toml::from_str(
            r#"values = { tag-digest = true }
helmfile = { oci-digest = false }
aliases = { stable = "https://charts.helm.sh/stable" }"#,
        )
        .unwrap();
        want.plugins.insert(
            "helm".into(),
            PluginLayer {
                run: Some(true),
                kinds: kinds(&["chart-dependency", "values-image", "helmfile-release"]),
                options: Some(options),
                ..PluginLayer::default()
            },
        );
        want.plugins.insert(
            "github-actions".into(),
            PluginLayer {
                run: Some(true),
                kinds: kinds(&["action", "docker-action", "container"]),
                ..PluginLayer::default()
            },
        );
        assert_eq!(parse_text(FULL), Ok(want));
        let block = parse_text("version = 1\n[release_age]\nunknown = \"block\"\nmin_days = 7\n");
        let block = block.unwrap();
        assert_eq!(block.unknown_age, Some(AgePolicy::Block));
        assert_eq!(block.min_age_days, Some(7));
    }

    /// `Exact` = the whole message; `Contains` = a toml/serde message.
    enum Want {
        Exact(&'static str),
        Contains(&'static str),
    }
    use Want::*;

    #[test]
    fn repo_rejects() {
        let cred = "plugins.helm.aliases.s: a repository URL must not contain credentials";
        let rows: Vec<(&str, Want)> = vec![
            ("[update]\nenabled = true\n", Exact("missing `version = 1`")),
            ("version = 2\n", Exact("unsupported version 2 (expected 1)")),
            (
                "version = 1\n[update]\nenable = true\n",
                Contains("unknown field `enable`"),
            ),
            (
                "version = 1\n[plugins.gitlab]\nrun = true\n",
                Exact("unknown plugin `gitlab` (known: gitlab-ci, helm, github-actions)"),
            ),
            (
                "version = 1\n[plugins.gitlab-ci]\nkinds = { images = true }\n",
                Exact("unknown kind `images` for plugin `gitlab-ci`"),
            ),
            (
                "version = 1\n[release_age]\nunknown = \"maybe\"\n",
                Contains("unknown variant `maybe`"),
            ),
            (
                "version = 1\n[plugins.gitlab-ci]\ngitlab-hosts = [\"https://x\"]\n",
                Exact("invalid GitLab host `https://x` in plugins.gitlab-ci.gitlab-hosts"),
            ),
            (
                "version = 1\n[plugins.gitlab-ci]\ngitlab-hosts = [\"\"]\n",
                Exact("invalid GitLab host `` in plugins.gitlab-ci.gitlab-hosts"),
            ),
            (
                "version = 1\n[plugins.gitlab-ci]\ngitlab-hosts = [\"a b\"]\n",
                Exact("invalid GitLab host `a b` in plugins.gitlab-ci.gitlab-hosts"),
            ),
            // Review-2 L6 (`pin_core::run::valid_gitlab_host`).
            (
                "version = 1\n[plugins.gitlab-ci]\ngitlab-hosts = [\"ok.example\", \"a@b\"]\n",
                Exact("invalid GitLab host `a@b` in plugins.gitlab-ci.gitlab-hosts"),
            ),
            (
                "version = 1\n[plugins.gitlab-ci]\ngitlab-hosts = [\"h:x\"]\n",
                Exact("invalid GitLab host `h:x` in plugins.gitlab-ci.gitlab-hosts"),
            ),
            (
                "version = 1\ngitlab-host = \"evil.example\"\n",
                Contains("unknown field `gitlab-host`"),
            ),
            (
                "version = 1\n[plugins.gitlab-ci]\ngitlab-host = \"evil.example\"\n",
                Contains("unknown field `gitlab-host`"),
            ),
            (
                "version = 1\n[plugins.helm]\naliases = { s = \"https://u:p@charts.example\" }\n",
                Exact(cred),
            ),
            (
                "version = 1\n[plugins.helm]\naliases = { s = \"https://tok@charts.example\" }\n",
                Exact(cred),
            ),
            (
                "version = 1\n[plugins.helm]\naliases = { s = 1 }\n",
                Contains("invalid type"),
            ),
            // Review-2 L2: globs of every plugin table, enabled or not.
            (
                "version = 1\n[plugins.gitlab-ci]\nrun = false\nfiles = [\"[\"]\n",
                Contains("invalid glob `[` in plugins.gitlab-ci.files: "),
            ),
            (
                "version = 1\n[plugins.helm]\nfiles = [\"*.yaml\", \"[\"]\n",
                Contains("invalid glob `[` in plugins.helm.files: "),
            ),
            (
                "version = 1\n[plugins.github-actions]\nfiles = [\"a{b\"]\n",
                Contains("invalid glob `a{b` in plugins.github-actions.files: "),
            ),
        ];
        for (text, want) in rows {
            let got = parse_text(text).expect_err(text).0;
            match want {
                Exact(m) => assert_eq!(got, format!("{PATH}: {m}"), "{text}"),
                Contains(m) => {
                    assert!(got.starts_with(&format!("{PATH}: ")), "{text}: {got}");
                    assert!(got.contains(m), "{text}: {got}");
                }
            }
        }
        let ok = "version = 1\n[plugins.helm]\naliases = { s = \"https://charts.example/a@b\" }\n";
        assert!(parse_text(ok).is_ok(), "`@` after the host is no userinfo");
    }

    #[test]
    fn repo_global_only_keys() {
        for (text, key) in [
            (
                "version = 1\n[hosts.\"gitlab.example.com\"]\nkind = \"gitlab\"\n",
                "hosts",
            ),
            ("version = 1\nconcurrency = 4\n", "concurrency"),
        ] {
            assert_eq!(
                parse_text(text),
                Err(Fatal(format!(
                    "{PATH}: `{key}` is a global-only key; it is not allowed in a repository config"
                ))),
                "{text}"
            );
        }
    }
}
