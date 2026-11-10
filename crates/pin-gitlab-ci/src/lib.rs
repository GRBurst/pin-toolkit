pub mod discover;
#[doc(hidden)]
pub mod driver;
mod policy;

#[doc(hidden)]
pub use driver::{PinOptions, client_from_env, exit_code, gitlab_from_env, pin_files};

use std::path::Path;

use crate::discover::{PinTarget, discover};
use pin_core::oci::pin_inline_image;
use pin_core::plugin::{Finding, KindSet, KindSpec, Plugin, PluginMeta, ScanError};
use pin_core::run::Ctx;

/// The reference kinds this plugin finds (all on by default).
pub const KINDS: [KindSpec; 3] = [
    KindSpec {
        id: "image",
        default_on: true,
        help: "image: and services: references (C1, C2)",
    },
    KindSpec {
        id: "component",
        default_on: true,
        help: "top-level component: references (C3)",
    },
    KindSpec {
        id: "include",
        default_on: true,
        help: "include: project/ref mappings (C4)",
    },
];

/// Files the plugin claims by default (globs).
pub const DEFAULT_FILES: [&str; 3] = [
    ".gitlab-ci.yml",
    "**/*.gitlab-ci.yml",
    ".gitlab/ci/**/*.yml",
];

/// The gitlab-ci plugin: `.gitlab-ci.yml` images, components and includes.
#[derive(Debug, Clone)]
pub struct GitlabCi {
    /// Component hosts that resolve; any other host is skipped before a
    /// client is built (host gate, spec §6.2).
    pub gitlab_hosts: Vec<String>,
}

impl Default for GitlabCi {
    fn default() -> Self {
        Self {
            gitlab_hosts: vec!["gitlab.com".to_string()],
        }
    }
}

impl PluginMeta for GitlabCi {
    fn id(&self) -> &'static str {
        "gitlab-ci"
    }

    fn kinds(&self) -> &'static [KindSpec] {
        &KINDS
    }

    fn default_files(&self) -> &'static [&'static str] {
        &DEFAULT_FILES
    }
}

impl Plugin for GitlabCi {
    type Target = PinTarget;

    /// `discover(text)` filtered by kind; never `Err` until M5c-y.
    fn scan(&self, _path: &Path, text: &str, on: &KindSet) -> Result<Vec<PinTarget>, ScanError> {
        Ok(discover(text)
            .into_iter()
            .filter(|t| on.contains(target_kind(t)))
            .collect())
    }

    async fn resolve(&self, ctx: &Ctx, text: &str, t: &PinTarget) -> Finding {
        match t {
            PinTarget::Image {
                line,
                start,
                end,
                value,
            } => pin_inline_image(ctx, text, *line, *start, *end, value, "image").await,
            PinTarget::Component { .. } => {
                policy::resolve_component(&self.gitlab_hosts, ctx, text, t).await
            }
            PinTarget::IncludeRef { .. } => policy::resolve_include(ctx, text, t).await,
            PinTarget::Unsupported { .. } => policy::unsupported(text, t),
        }
    }
}

/// The kind id of a discovered target.
fn target_kind(t: &PinTarget) -> &'static str {
    match t {
        PinTarget::Image { .. } => "image",
        PinTarget::Component { .. } => "component",
        PinTarget::IncludeRef { .. } => "include",
        PinTarget::Unsupported { kind, .. } => kind,
    }
}
