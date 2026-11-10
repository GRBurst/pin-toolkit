pub mod discover;
mod policy;

use std::path::Path;

use crate::discover::{PinTarget, discover};
use pin_core::oci::pin_inline_image;
use pin_core::plugin::{Finding, KindSet, KindSpec, Plugin, PluginMeta, ScanError, Source};
use pin_core::run::Ctx;

/// Kind id of `image:` and `services:` references (review L10).
pub const IMAGE: &str = "image";
/// Kind id of top-level `component:` references.
pub const COMPONENT: &str = "component";
/// Kind id of classic `include:` project/ref mappings.
pub const INCLUDE: &str = "include";

/// The reference kinds this plugin finds (all on by default).
pub const KINDS: [KindSpec; 3] = [
    KindSpec {
        id: IMAGE,
        default_on: true,
        help: "image: and services: references (C1, C2)",
    },
    KindSpec {
        id: COMPONENT,
        default_on: true,
        help: "top-level component: references (C3)",
    },
    KindSpec {
        id: INCLUDE,
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
/// Component hosts that resolve come from the run's one GitLab host set
/// ([`Ctx::gitlab_enabled`], review L9); any other host is skipped before a
/// client is built (host gate, spec §6.2).
#[derive(Debug, Clone, Copy, Default)]
pub struct GitlabCi;

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

    /// `discover(text)` filtered by kind. `Err` = the file is not valid YAML
    /// (§6.9): no partial targets.
    fn scan(&self, _path: &Path, text: &str, on: &KindSet) -> Result<Vec<PinTarget>, ScanError> {
        Ok(discover(text)?
            .into_iter()
            .filter(|t| on.contains(target_kind(t)))
            .collect())
    }

    /// `ignore` key (T-D17): `{project}@{ref}` for an include, else the value
    /// with quotes stripped (`alpine:3.20` does not match a pinned
    /// `alpine:3.20@sha256:…`; `alpine:3.20*` does).
    fn ignore_key(&self, t: &PinTarget) -> Option<String> {
        Some(match t {
            PinTarget::IncludeRef { project, value, .. } => format!("{project}@{value}"),
            _ => t.value().to_string(),
        })
    }

    async fn resolve(&self, ctx: &Ctx, src: &Source<'_>, t: &PinTarget) -> Finding {
        match t {
            PinTarget::Image {
                line,
                start,
                end,
                value,
            } => pin_inline_image(ctx, src.text, *line, *start, *end, value, IMAGE).await,
            PinTarget::Component { parts, .. } => {
                policy::resolve_component(ctx, src, t, parts.as_ref()).await
            }
            PinTarget::IncludeRef { project, .. } => {
                policy::resolve_include(ctx, src, t, project).await
            }
            PinTarget::Unsupported { kind, reason, .. } => {
                policy::unsupported(src, t, kind, reason)
            }
        }
    }
}

/// The kind id of a discovered target.
fn target_kind(t: &PinTarget) -> &'static str {
    match t {
        PinTarget::Image { .. } => IMAGE,
        PinTarget::Component { .. } => COMPONENT,
        PinTarget::IncludeRef { .. } => INCLUDE,
        PinTarget::Unsupported { kind, .. } => kind,
    }
}
