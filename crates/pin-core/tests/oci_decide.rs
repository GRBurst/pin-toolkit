//! `oci::decide_image` / `pin_inline_image` (M5c C8): today's image policy in
//! core, reached only through the `Ctx` entry points (D38).

use std::collections::HashMap;

use pin_core::edit::Splice;
use pin_core::oci::{ImageDecision, ImageRef, RegistryClient, decide_image, pin_inline_image};
use pin_core::report::Status;
use pin_core::run::{Ctx, ResolverEnv, UpdatePolicy};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const D2: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

fn head(repo_tag: &str, status: u16, digest: Option<&str>) -> Mock {
    let (repo, tag) = repo_tag.rsplit_once(':').unwrap();
    let mut resp = ResponseTemplate::new(status);
    if let Some(d) = digest {
        resp = resp.insert_header("docker-content-digest", d);
    }
    Mock::given(method("HEAD"))
        .and(path(format!("/v2/{repo}/manifests/{tag}")))
        .respond_with(resp)
}

fn tag_list(repo: &str, tags: &[&str]) -> Mock {
    Mock::given(method("GET"))
        .and(path(format!("/v2/{repo}/tags/list")))
        .and(query_param("n", "1000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"tags": tags})))
}

/// alpine 3.20 → D1, 3.24 → D2 (tags 3.16/3.20/3.24); mystery:1.0 → 404.
async fn mock_server() -> MockServer {
    let s = MockServer::start().await;
    head("library/alpine:3.20", 200, Some(D1)).mount(&s).await;
    head("library/alpine:3.24", 200, Some(D2)).mount(&s).await;
    head("library/mystery:1.0", 404, None).mount(&s).await;
    tag_list("library/alpine", &["3.16", "3.20", "3.24"])
        .mount(&s)
        .await;
    s
}

fn ctx(s: &MockServer, update: UpdatePolicy) -> Ctx {
    let client = RegistryClient::new(
        Default::default(),
        HashMap::from([
            ("docker.io".to_string(), s.uri()),
            ("ghcr.io".to_string(), s.uri()),
        ]),
    );
    Ctx::new(update, 0, ResolverEnv::default()).with_oci(Ok(client))
}

fn update_on() -> UpdatePolicy {
    UpdatePolicy {
        enabled: true,
        ..Default::default()
    }
}

fn parse(s: &str) -> ImageRef {
    ImageRef::parse(s).unwrap()
}

#[tokio::test]
async fn decide_pin_unpinned() {
    let s = mock_server().await;
    let d = decide_image(&ctx(&s, Default::default()), &parse("alpine:3.20")).await;
    assert_eq!(
        d,
        Ok(ImageDecision::Pin {
            digest: D1.to_string()
        })
    );
}

#[tokio::test]
async fn decide_current_when_digest_matches() {
    let s = mock_server().await;
    let r = parse(&format!("alpine:3.20@{D1}"));
    let d = decide_image(&ctx(&s, Default::default()), &r).await;
    assert_eq!(d, Ok(ImageDecision::Current));
}

#[tokio::test]
async fn decide_update_bumps_to_latest_stable() {
    let s = mock_server().await;
    let d = decide_image(&ctx(&s, update_on()), &parse("alpine:3.20")).await;
    assert_eq!(
        d,
        Ok(ImageDecision::Update {
            tag: "3.24".to_string(),
            digest: D2.to_string()
        })
    );
}

#[tokio::test]
async fn decide_tag_list_error_is_prefixed() {
    let s = MockServer::start().await;
    head("library/alpine:3.20", 200, Some(D1)).mount(&s).await;
    Mock::given(method("GET"))
        .and(path("/v2/library/alpine/tags/list"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&s)
        .await;
    let d = decide_image(&ctx(&s, update_on()), &parse("alpine:3.20")).await;
    let err = d.unwrap_err();
    assert!(err.starts_with("tag list: "), "{err}");
}

#[tokio::test]
async fn decide_digest_error_is_inner_message() {
    let s = mock_server().await;
    let d = decide_image(&ctx(&s, Default::default()), &parse("mystery:1.0")).await;
    assert_eq!(d, Err("library/mystery:1.0 not found".to_string()));
}

#[tokio::test]
async fn inline_unparseable_builds_no_client() {
    let ctx = Ctx::new(Default::default(), 0, ResolverEnv::default());
    let text = "image: ${X}:${Y}\n";
    let f = pin_inline_image(&ctx, text, 0, 7, 16, "${X}:${Y}", "image").await;
    assert_eq!(f.status, Status::Skipped);
    assert_eq!(f.reason.as_deref(), Some("unparseable reference"));
    assert!(f.edits.is_empty());
    assert_eq!(ctx.resolvers_built(), Vec::<&str>::new());
}

#[tokio::test]
async fn inline_pin_single_splice_keeps_quotes() {
    let s = mock_server().await;
    let text = "image: \"alpine:3.20\"\n";
    let f = pin_inline_image(
        &ctx(&s, Default::default()),
        text,
        0,
        7,
        20,
        "alpine:3.20",
        "service",
    )
    .await;
    let token = format!("\"alpine:3.20@{D1}\"");
    assert_eq!(f.status, Status::Pinned);
    assert_eq!(f.edits, [Splice::replace(7, 20, token.clone())]);
    assert_eq!(f.new, Some(token));
    assert_eq!(f.kind, "service");
    assert_eq!(f.old, "\"alpine:3.20\"");
    assert_eq!(f.reference, "alpine:3.20");
    assert_eq!(f.line, 0);
}

#[tokio::test]
async fn inline_construction_error_is_unresolved() {
    let ctx = Ctx::new(Default::default(), 0, ResolverEnv::default()).with_oci(Err("ca: x".into()));
    let text = "image: alpine:3.20\n";
    let f = pin_inline_image(&ctx, text, 0, 7, 18, "alpine:3.20", "image").await;
    assert_eq!(f.status, Status::Unresolved);
    assert_eq!(f.reason.as_deref(), Some("ca: x"));
    assert!(f.edits.is_empty());
}
