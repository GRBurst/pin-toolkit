//! `GitlabCi` plugin contract (M5c C9): pure kind-filtered scan, identity,
//! host gate before any client, include policy through `Ctx`.

mod common;

use std::path::Path;

use common::*;
use pin_core::edit::Splice;
use pin_core::plugin::{DynPlugin, KindSet, Plugin, PluginMeta};
use pin_core::report::Status;
use pin_core::run::{Ctx, ResolverEnv};
use pin_gitlab_ci::discover::PinTarget;
use pin_gitlab_ci::{DEFAULT_FILES, GitlabCi, KINDS};

fn env() -> ResolverEnv {
    ResolverEnv {
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    }
}

#[test]
fn scan_filters_by_kind() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ci-rich.yml"
    ))
    .unwrap();
    let p = GitlabCi::default();
    let path = Path::new("ci-rich.yml");

    let on = KindSet::defaults(&KINDS);
    assert_eq!(p.scan(path, &text, &on).unwrap().len(), 7);

    let mut no_include = KindSet::defaults(&KINDS);
    no_include.set("include", false);
    let targets = p.scan(path, &text, &no_include).unwrap();
    assert_eq!(targets.len(), 6);
    assert!(
        !targets
            .iter()
            .any(|t| matches!(t, PinTarget::IncludeRef { .. }))
    );

    let mut no_image = KindSet::defaults(&KINDS);
    no_image.set("image", false);
    assert_eq!(p.scan(path, &text, &no_image).unwrap().len(), 1);
}

#[test]
fn ids_kinds_and_files() {
    // `Plugin`, `DynPlugin` and `PluginMeta` all in scope: plain method
    // calls resolve to the one `PluginMeta` method set (P22).
    fn as_dyn(p: &dyn DynPlugin) -> &'static str {
        p.id()
    }
    let p = GitlabCi::default();
    assert_eq!(p.id(), "gitlab-ci");
    assert_eq!(as_dyn(&p), "gitlab-ci");
    assert_eq!(
        p.kinds().iter().map(|k| k.id).collect::<Vec<_>>(),
        ["image", "component", "include"]
    );
    assert!(p.kinds().iter().all(|k| k.default_on));
    assert_eq!(p.default_files(), &DEFAULT_FILES[..]);
    assert_eq!(p.gitlab_hosts, ["gitlab.com"]);
}

#[tokio::test]
async fn component_other_host_builds_no_client() {
    let p = GitlabCi::default();
    let ctx = Ctx::new(Default::default(), 0, env());
    let text = "component: example.org/g/p/c@1.0.0\n";
    let targets = p
        .scan(Path::new("x.yml"), text, &KindSet::defaults(&KINDS))
        .unwrap();
    assert_eq!(targets.len(), 1);
    let f = p.resolve(&ctx, text, &targets[0]).await;
    assert_eq!(f.status, Status::Skipped);
    assert_eq!(f.reason.as_deref(), Some("host not enabled: example.org"));
    assert_eq!(f.kind, "component");
    assert!(f.edits.is_empty());
    assert_eq!(ctx.resolvers_built(), Vec::<&str>::new());
}

#[tokio::test]
async fn include_pin_has_replacement_and_comment() {
    let fakes = start_fakes().await;
    let p = GitlabCi::default();
    let ctx =
        Ctx::new(Default::default(), 0, env()).with_gitlab("gitlab.com", Ok(gitlab_client(&fakes)));
    let text = "include:\n  project: g/p\n  ref: main\n";
    let targets = p
        .scan(Path::new("x.yml"), text, &KindSet::defaults(&KINDS))
        .unwrap();
    assert_eq!(targets.len(), 1);
    let f = p.resolve(&ctx, text, &targets[0]).await;
    let sha = fake_hex("g%2Fp@main", 40);
    let start = text.find("main").unwrap();
    let line_end = text.len() - 1;
    assert_eq!(f.status, Status::Pinned, "{:?}", f.reason);
    assert_eq!(f.kind, "include");
    assert_eq!(f.new.as_deref(), Some(sha.as_str()));
    assert_eq!(
        f.edits,
        [
            Splice::replace(start, start + 4, sha.clone()),
            Splice::insert(line_end, "  # main"),
        ]
    );
}

/// A reference at a target path that cannot be spliced (block scalar) is
/// reported `skipped (<reason>)`; no resolver is built and nothing is edited.
#[tokio::test]
async fn unsupported_target_is_skipped_without_client() {
    let p = GitlabCi::default();
    let ctx = Ctx::new(Default::default(), 0, env());
    let text = "image: >-\n  alpine:3.20\n";
    let t = PinTarget::Unsupported {
        kind: "image",
        line: 1,
        start: 12,
        end: 23,
        value: "alpine:3.20".into(),
        reason: "block scalar",
    };
    let f = p.resolve(&ctx, text, &t).await;
    assert_eq!(f.status, Status::Skipped);
    assert_eq!(f.reason.as_deref(), Some("block scalar"));
    assert_eq!(f.kind, "image");
    assert_eq!(f.old, "alpine:3.20");
    assert_eq!(f.line, 1);
    assert!(f.edits.is_empty());
    assert_eq!(ctx.resolvers_built(), Vec::<&str>::new());
}
