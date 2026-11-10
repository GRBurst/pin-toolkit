//! Every plugin id the toolkit knows (O1), compiled in or not, and the
//! plugins this build contains (D9).

use pin_core::plugin::DynPlugin;
#[cfg(feature = "gitlab-ci")]
use pin_gitlab_ci::GitlabCi;

/// A known plugin: its id and every kind id it has or will have.
pub struct Known {
    pub id: &'static str,
    pub kinds: &'static [&'static str],
}

/// The static list of known plugins (main: anything else is exit 2).
/// Config tables and `--kind` values are validated against it, so a repo
/// config written for a larger build still loads.
pub const KNOWN: [Known; 3] = [
    Known {
        id: "gitlab-ci",
        kinds: &["image", "service", "component", "include", "helm-oci"],
    },
    Known {
        id: "helm",
        kinds: &["chart-dependency", "values-image", "helmfile-release"],
    },
    Known {
        id: "github-actions",
        kinds: &["action", "docker-action", "container"],
    },
];

/// argv[0] names that are not `pin-<id>` but dispatch to a plugin (D33:
/// `pin-ci` is the one entry).
pub const ALIASES: [(&str, &str); 1] = [("pin-ci", "gitlab-ci")];

/// The known plugin `id`.
pub fn known(id: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.id == id)
}

#[cfg(feature = "gitlab-ci")]
static GITLAB_CI: GitlabCi = GitlabCi;

/// The plugins compiled into this build, in `KNOWN` order.
// One cfg'd push per plugin feature (none without features).
#[allow(unused_mut, clippy::vec_init_then_push)]
pub fn compiled() -> Vec<&'static dyn DynPlugin> {
    let mut v: Vec<&'static dyn DynPlugin> = Vec::new();
    #[cfg(feature = "gitlab-ci")]
    v.push(&GITLAB_CI);
    v
}

/// The compiled plugin `id`.
pub fn compiled_plugin(id: &str) -> Option<&'static dyn DynPlugin> {
    compiled().into_iter().find(|p| p.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P-known: every compiled plugin is known, with known kinds only.
    #[test]
    fn compiled_plugins_are_known() {
        for p in compiled() {
            let k = known(p.id()).unwrap_or_else(|| panic!("{} not in KNOWN", p.id()));
            for kind in p.kinds() {
                assert!(
                    k.kinds.contains(&kind.id),
                    "{}: kind {} not in KNOWN",
                    p.id(),
                    kind.id
                );
            }
        }
    }
}
