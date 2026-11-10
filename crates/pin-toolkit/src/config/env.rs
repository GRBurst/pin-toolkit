//! The env layer: `PIN_TOOLKIT_*` (spec §7.1, T-D7). Empty values count
//! as unset; a bad value of a known variable is fatal; any other
//! `PIN_TOOLKIT_*` variable is only reported ([`unknown_vars`]), because
//! GitLab exports every CI/CD variable to the job (PR-M3).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

use pin_core::run::AgePolicy;

use super::{Fatal, Layer, valid_gitlab_host};

/// Prefix of the toolkit's variables.
pub const PREFIX: &str = "PIN_TOOLKIT_";

pub const UPDATE: &str = "PIN_TOOLKIT_UPDATE";
pub const UPDATE_MAJOR: &str = "PIN_TOOLKIT_UPDATE_MAJOR";
pub const MIN_AGE_DAYS: &str = "PIN_TOOLKIT_MIN_AGE_DAYS";
pub const RELEASE_AGE_UNKNOWN: &str = "PIN_TOOLKIT_RELEASE_AGE_UNKNOWN";
pub const CONCURRENCY: &str = "PIN_TOOLKIT_CONCURRENCY";
pub const JSON: &str = "PIN_TOOLKIT_JSON";
pub const ONLY: &str = "PIN_TOOLKIT_ONLY";
pub const SKIP: &str = "PIN_TOOLKIT_SKIP";
pub const GITLAB_HOST: &str = "PIN_TOOLKIT_GITLAB_HOST";
/// The repo config path (below `--config`); not a [`Layer`] field.
pub const CONFIG: &str = "PIN_TOOLKIT_CONFIG";

/// Every variable the toolkit reads.
pub const KNOWN_VARS: [&str; 10] = [
    UPDATE,
    UPDATE_MAJOR,
    MIN_AGE_DAYS,
    RELEASE_AGE_UNKNOWN,
    CONCURRENCY,
    JSON,
    ONLY,
    SKIP,
    GITLAB_HOST,
    CONFIG,
];

/// `vars[name]` unless missing or empty.
fn get<'a>(vars: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    vars.get(name).map(String::as_str).filter(|v| !v.is_empty())
}

fn boolean(vars: &BTreeMap<String, String>, name: &str) -> Result<Option<bool>, Fatal> {
    let Some(v) = get(vars, name) else {
        return Ok(None);
    };
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => Err(Fatal(format!(
            "{name}: expected a boolean (1/0, true/false, yes/no, on/off), got \"{v}\""
        ))),
    }
}

fn number<T: FromStr>(vars: &BTreeMap<String, String>, name: &str) -> Result<Option<T>, Fatal> {
    let Some(v) = get(vars, name) else {
        return Ok(None);
    };
    v.parse()
        .map(Some)
        .map_err(|_| Fatal(format!("{name}: expected a number, got \"{v}\"")))
}

fn age(vars: &BTreeMap<String, String>, name: &str) -> Result<Option<AgePolicy>, Fatal> {
    let Some(v) = get(vars, name) else {
        return Ok(None);
    };
    match v.to_ascii_lowercase().as_str() {
        "allow" => Ok(Some(AgePolicy::Allow)),
        "block" => Ok(Some(AgePolicy::Block)),
        _ => Err(Fatal(format!(
            "{name}: expected allow or block, got \"{v}\""
        ))),
    }
}

/// A comma list; items trimmed, empty items dropped. Ids are checked where
/// they are used (enablement), so a value that does not apply to a run
/// cannot fail it.
fn list(vars: &BTreeMap<String, String>, name: &str) -> Option<Vec<String>> {
    get(vars, name).map(|v| {
        v.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    })
}

/// The env layer from the known variables; every other variable is
/// ignored here.
pub fn layer(vars: &BTreeMap<String, String>) -> Result<Layer, Fatal> {
    let gitlab_host = get(vars, GITLAB_HOST).map(String::from);
    if let Some(h) = &gitlab_host
        && !valid_gitlab_host(h)
    {
        return Err(Fatal(format!("{GITLAB_HOST}: invalid GitLab host `{h}`")));
    }
    Ok(Layer {
        update: boolean(vars, UPDATE)?,
        update_major: boolean(vars, UPDATE_MAJOR)?,
        min_age_days: number(vars, MIN_AGE_DAYS)?,
        unknown_age: age(vars, RELEASE_AGE_UNKNOWN)?,
        concurrency: number(vars, CONCURRENCY)?,
        json: boolean(vars, JSON)?,
        only: list(vars, ONLY),
        skip: list(vars, SKIP),
        gitlab_host,
        ..Layer::default()
    })
}

/// `PIN_TOOLKIT_*` variables the toolkit does not read, sorted: one
/// warning each, exit code unaffected (PR-M3).
pub fn unknown_vars(vars: &BTreeMap<String, String>) -> Vec<String> {
    vars.keys()
        .filter(|k| k.starts_with(PREFIX) && !KNOWN_VARS.contains(&k.as_str()))
        .cloned()
        .collect()
}

/// `PIN_TOOLKIT_CONFIG`, if set.
pub fn config_path(vars: &BTreeMap<String, String>) -> Option<PathBuf> {
    get(vars, CONFIG).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn one(name: &str, v: &str) -> Result<Layer, Fatal> {
        layer(&vars(&[(name, v)]))
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn env_vars_map_to_fields() {
        let rows: Vec<(&str, &str, Layer)> = vec![
            (
                UPDATE,
                "1",
                Layer {
                    update: Some(true),
                    ..Layer::default()
                },
            ),
            (
                UPDATE_MAJOR,
                "true",
                Layer {
                    update_major: Some(true),
                    ..Layer::default()
                },
            ),
            (
                MIN_AGE_DAYS,
                "14",
                Layer {
                    min_age_days: Some(14),
                    ..Layer::default()
                },
            ),
            (
                RELEASE_AGE_UNKNOWN,
                "block",
                Layer {
                    unknown_age: Some(AgePolicy::Block),
                    ..Layer::default()
                },
            ),
            (
                RELEASE_AGE_UNKNOWN,
                "allow",
                Layer {
                    unknown_age: Some(AgePolicy::Allow),
                    ..Layer::default()
                },
            ),
            (
                CONCURRENCY,
                "3",
                Layer {
                    concurrency: Some(3),
                    ..Layer::default()
                },
            ),
            (
                JSON,
                "yes",
                Layer {
                    json: Some(true),
                    ..Layer::default()
                },
            ),
            (
                ONLY,
                "gitlab-ci, helm",
                Layer {
                    only: Some(strings(&["gitlab-ci", "helm"])),
                    ..Layer::default()
                },
            ),
            (
                SKIP,
                "helm,",
                Layer {
                    skip: Some(strings(&["helm"])),
                    ..Layer::default()
                },
            ),
            (
                GITLAB_HOST,
                "gitlab.example.com",
                Layer {
                    gitlab_host: Some("gitlab.example.com".into()),
                    ..Layer::default()
                },
            ),
            (CONFIG, "x.toml", Layer::default()),
        ];
        for (name, v, want) in rows {
            assert_eq!(one(name, v), Ok(want), "{name}={v}");
            assert_eq!(one(name, ""), Ok(Layer::default()), "{name}= is unset");
        }
        for name in [UPDATE, UPDATE_MAJOR, JSON] {
            for (v, b) in [
                ("1", true),
                ("TRUE", true),
                ("Yes", true),
                ("on", true),
                ("0", false),
                ("False", false),
                ("NO", false),
                ("Off", false),
            ] {
                let l = one(name, v).unwrap();
                let got = [l.update, l.update_major, l.json];
                assert_eq!(got.iter().flatten().collect::<Vec<_>>(), [&b], "{name}={v}");
            }
        }
    }

    #[test]
    fn env_rejects_bad_values() {
        let boolean = "expected a boolean (1/0, true/false, yes/no, on/off)";
        let rows = [
            (
                UPDATE,
                "maybe",
                format!("{UPDATE}: {boolean}, got \"maybe\""),
            ),
            (
                UPDATE_MAJOR,
                "2",
                format!("{UPDATE_MAJOR}: {boolean}, got \"2\""),
            ),
            (JSON, "y", format!("{JSON}: {boolean}, got \"y\"")),
            (
                MIN_AGE_DAYS,
                "-1",
                format!("{MIN_AGE_DAYS}: expected a number, got \"-1\""),
            ),
            (
                CONCURRENCY,
                "many",
                format!("{CONCURRENCY}: expected a number, got \"many\""),
            ),
            (
                RELEASE_AGE_UNKNOWN,
                "maybe",
                format!("{RELEASE_AGE_UNKNOWN}: expected allow or block, got \"maybe\""),
            ),
            (
                GITLAB_HOST,
                "https://x",
                format!("{GITLAB_HOST}: invalid GitLab host `https://x`"),
            ),
            (
                GITLAB_HOST,
                "a b",
                format!("{GITLAB_HOST}: invalid GitLab host `a b`"),
            ),
        ];
        for (name, v, want) in rows {
            assert_eq!(one(name, v), Err(Fatal(want)), "{name}={v}");
        }
        assert_eq!(
            layer(&vars(&[("UPDATE", "maybe"), ("CI_JOB_TOKEN", "x")])),
            Ok(Layer::default()),
            "variables without the prefix are not read"
        );
    }

    #[test]
    fn env_unknown_vars_warn() {
        let v = vars(&[
            ("PIN_TOOLKIT_FOO", "1"),
            ("PIN_TOOLKIT_VERSION", "0.2"),
            (UPDATE, "1"),
            (CONFIG, "x.toml"),
            ("CI_JOB_TOKEN", "x"),
        ]);
        assert_eq!(
            layer(&v),
            Ok(Layer {
                update: Some(true),
                ..Layer::default()
            })
        );
        assert_eq!(unknown_vars(&v), ["PIN_TOOLKIT_FOO", "PIN_TOOLKIT_VERSION"]);
    }

    #[test]
    fn env_config_path() {
        assert_eq!(
            config_path(&vars(&[(CONFIG, "ci/x.toml")])),
            Some("ci/x.toml".into())
        );
        assert_eq!(config_path(&vars(&[(CONFIG, "")])), None);
        assert_eq!(config_path(&vars(&[])), None);
    }
}
