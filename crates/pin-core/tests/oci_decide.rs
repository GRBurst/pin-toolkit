//! `oci::decide_image` / `pin_inline_image` (M5c C8): today's image policy in
//! core, reached only through the `Ctx` entry points (D38).

use std::collections::HashMap;

use pin_core::edit::Splice;
use pin_core::oci::{ImageDecision, ImageRef, RegistryClient, decide_image, pin_inline_image};
use pin_core::report::Status;
use pin_core::run::{AgePolicy, Ctx, ResolverEnv, UpdatePolicy};
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

/// ghcr.io `org/img` (tags v1, v2: v1 → D1, v2 → D2), the update policy
/// `enabled, allow_major` (v1 → v2 is a major bump), `min_age_days = 7`,
/// `unknown_age`, and a Hub API fake that must
/// never be asked (`.expect(0)`: tag times come only from the Hub API).
async fn ghcr_update(unknown_age: AgePolicy) -> (Result<ImageDecision, String>, MockServer) {
    let s = MockServer::start().await;
    head("org/img:v1", 200, Some(D1)).mount(&s).await;
    head("org/img:v2", 200, Some(D2)).mount(&s).await;
    tag_list("org/img", &["v1", "v2"]).mount(&s).await;
    let hub = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&hub)
        .await;
    let client = RegistryClient::new(
        Default::default(),
        HashMap::from([("ghcr.io".to_string(), s.uri())]),
    )
    .with_hub_base(&hub.uri());
    let update = UpdatePolicy {
        enabled: true,
        allow_major: true,
        min_age_days: 7,
        unknown_age,
    };
    let ctx = Ctx::new(update, 0, ResolverEnv::default()).with_oci(Ok(client));
    let d = decide_image(&ctx, &parse("ghcr.io/org/img:v1")).await;
    (d, hub)
}

/// §6.8: `release_age.unknown = block` — a non-docker.io host has no
/// timestamps, so an update is skipped with its own reason; no Hub or
/// tag-time request is made.
#[tokio::test]
async fn unknown_age_block_skips_non_hub_updates() {
    let (d, hub) = ghcr_update(AgePolicy::Block).await;
    assert_eq!(
        d,
        Ok(ImageDecision::Skip(
            "release age unknown (ghcr.io; release_age.unknown = block)".to_string()
        ))
    );
    hub.verify().await;
}

/// `allow` (the default) keeps today's no-op guard for non-docker.io hosts.
#[tokio::test]
async fn unknown_age_allow_keeps_todays_update() {
    let (d, hub) = ghcr_update(AgePolicy::Allow).await;
    assert_eq!(
        d,
        Ok(ImageDecision::Update {
            tag: "v2".to_string(),
            digest: D2.to_string()
        })
    );
    hub.verify().await;
}
