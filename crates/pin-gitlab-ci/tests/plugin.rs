//! `GitlabCi` plugin contract (M5c C9): pure kind-filtered scan, identity,
//! host gate before any client, include policy through `Ctx`.

mod common;

use std::path::Path;

use common::*;
use pin_core::edit::Splice;
use pin_core::plugin::{DynPlugin, KindSet, Plugin, PluginMeta, ScanError};
use pin_core::report::Status;
use pin_core::run::{Ctx, FileError, ResolverEnv, RunOptions, pin_file};
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

/// Review H1: tab separators after `key:` do not change what is pinned; the
/// edits index the original text.
#[tokio::test]
async fn include_pin_with_tab_separators() {
    let fakes = start_fakes().await;
    let p = GitlabCi::default();
    let ctx =
        Ctx::new(Default::default(), 0, env()).with_gitlab("gitlab.com", Ok(gitlab_client(&fakes)));
    let text = "include:\n  project:\tg/p\n  ref:\tmain\n";
    let targets = p
        .scan(Path::new("x.yml"), text, &KindSet::defaults(&KINDS))
        .unwrap();
    assert_eq!(targets.len(), 1);
    let f = p.resolve(&ctx, text, &targets[0]).await;
    let sha = fake_hex("g%2Fp@main", 40);
    let start = text.find("main").unwrap();
    assert_eq!(f.status, Status::Pinned, "{:?}", f.reason);
    assert_eq!(
        f.edits,
        [
            Splice::replace(start, start + 4, sha),
            Splice::insert(text.len() - 1, "  # main"),
        ]
    );
}

/// Review M2: a trailing comment must not land inside a multi-line sibling
/// scalar of a flow include; that include is `skipped (no safe place for
/// comment)` and untouched, the rest of the file is still pinned, and the
/// result still parses.
#[tokio::test]
async fn include_without_a_safe_comment_place_is_skipped() {
    let fakes = start_fakes().await;
    let dir = std::path::PathBuf::from(format!(".tmp/plugin-nocomment-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, nl) in [("lf.yml", "\n"), ("crlf.yml", "\r\n")] {
        let path = dir.join(name);
        let text = [
            "include:",
            "  - {project: g/p, ref: main, file: a",
            "      .yml}",
            "  - {project: g/r, ref: dev, file: \"b\\",
            "      .yml\"}",
            "  - project: g/q",
            "    ref: dev",
            "",
        ]
        .join(nl);
        std::fs::write(&path, &text).unwrap();
        let ctx = Ctx::new(Default::default(), 0, env())
            .with_gitlab("gitlab.com", Ok(gitlab_client(&fakes)));
        let opts = RunOptions {
            dry_run: false,
            check: false,
            concurrency: 4,
        };
        let rows = pin_file(
            &GitlabCi::default(),
            &ctx,
            &path,
            &KindSet::defaults(&KINDS),
            &opts,
        )
        .await
        .unwrap();
        let got: Vec<(usize, Status, Option<&str>)> = rows
            .iter()
            .map(|r| (r.line, r.status, r.reason.as_deref()))
            .collect();
        let no_place = Some("no safe place for comment");
        assert_eq!(
            got,
            [
                (2, Status::Skipped, no_place),
                (4, Status::Skipped, no_place),
                (7, Status::Pinned, None),
            ],
            "{name}"
        );
        let out = std::fs::read_to_string(&path).unwrap();
        let sha = fake_hex("g%2Fq@dev", 40);
        // only the safe include changed: its ref and its comment
        let want = text.replace("ref: dev\n", &format!("ref: {sha}  # dev\n"));
        let want = want.replace("ref: dev\r\n", &format!("ref: {sha}  # dev\r\n"));
        assert_eq!(out, want, "{name}");
        // the pinned file still parses
        assert!(pin_gitlab_ci::discover::discover(&out).is_ok(), "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
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

/// §6.9: a file that is not valid YAML is a scan error with the parser's
/// line (1-based) — no partial targets, no fallback to line scanning.
#[test]
fn scan_invalid_yaml_is_scan_error() {
    let p = GitlabCi::default();
    let on = KindSet::defaults(&KINDS);
    assert_eq!(
        p.scan(Path::new("x.yml"), "a: 1\nb: c: d\n", &on),
        Err(ScanError {
            line: Some(2),
            msg: "invalid YAML: mapping values are not allowed in this context".into()
        })
    );
    let err = p
        .scan(Path::new("x.yml"), "job:\n\timage: alpine:3.20\n", &on)
        .unwrap_err();
    assert_eq!(err.line, Some(2));
    assert!(err.msg.starts_with("invalid YAML: "), "{}", err.msg);
}

/// A parse error becomes `FileError::Parse` with path and line; nothing is
/// written and no resolver is built.
#[tokio::test]
async fn invalid_yaml_file_is_parse_error_and_untouched() {
    let dir = std::path::PathBuf::from(format!(".tmp/plugin-badyaml-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ci.yml");
    let text = "image: alpine:3.20\nb: c: d\n";
    std::fs::write(&path, text).unwrap();

    let ctx = Ctx::new(Default::default(), 0, env());
    let opts = RunOptions {
        dry_run: false,
        check: false,
        concurrency: 4,
    };
    let err = pin_file(
        &GitlabCi::default(),
        &ctx,
        &path,
        &KindSet::defaults(&KINDS),
        &opts,
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        FileError::Parse {
            path: path.display().to_string(),
            line: Some(2),
            msg: "invalid YAML: mapping values are not allowed in this context".into()
        }
    );
    assert!(
        err.to_string()
            .ends_with(":2: invalid YAML: mapping values are not allowed in this context"),
        "{err}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert_eq!(ctx.resolvers_built(), Vec::<&str>::new());
    let _ = std::fs::remove_dir_all(&dir);
}
