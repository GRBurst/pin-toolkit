//! Per-run resolver memo in `Ctx` (spec §6.7, D19): one request per key
//! within a run, errors cached, concurrent identical lookups coalesced.
//! Responses are delayed so the concurrent calls overlap; the mocks'
//! `expect(n)` is verified when the server drops.

use std::collections::HashMap;
use std::time::Duration;

use pin_core::gitlab::GitLabClient;
use pin_core::oci::ImageRef;
use pin_core::oci::registry::{Creds, RegistryClient};
use pin_core::run::{Ctx, ResolverEnv};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OTHER: &str = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
const SHA: &str = "e3262fdd0914fa823210cdb79a8c421e2cef79d8";
const DELAY: Duration = Duration::from_millis(100);

fn oci_ctx(server: &MockServer) -> Ctx {
    let client = RegistryClient::new(
        Creds::none(),
        HashMap::from([("docker.io".to_string(), server.uri())]),
    );
    Ctx::new(Default::default(), 0, ResolverEnv::default()).with_oci(Ok(client))
}

async fn mount_digest(server: &MockServer, tag: &str, code: u16, digest: &str) {
    Mock::given(method("HEAD"))
        .and(path(format!("/v2/library/alpine/manifests/{tag}")))
        .respond_with(
            ResponseTemplate::new(code)
                .insert_header("docker-content-digest", digest)
                .set_delay(DELAY),
        )
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn concurrent_digest_lookups_are_one_request() {
    let server = MockServer::start().await;
    mount_digest(&server, "3.20", 200, DIGEST).await;
    let ctx = oci_ctx(&server);
    let r = ImageRef::parse("alpine:3.20").unwrap();
    let (a, b) = tokio::join!(ctx.oci_digest(&r), ctx.oci_digest(&r));
    assert_eq!(a, Ok(DIGEST.to_string()));
    assert_eq!(b, Ok(DIGEST.to_string()));
}

#[tokio::test]
async fn failed_lookups_are_cached() {
    let server = MockServer::start().await;
    mount_digest(&server, "3.20", 404, DIGEST).await;
    let ctx = oci_ctx(&server);
    let r = ImageRef::parse("alpine:3.20").unwrap();
    let (a, b) = tokio::join!(ctx.oci_digest(&r), ctx.oci_digest(&r));
    let c = ctx.oci_digest(&r).await;
    assert!(a.is_err(), "{a:?}");
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[tokio::test]
async fn concurrent_gitlab_lookups_are_one_request() {
    let server = MockServer::start().await;
    for tag in ["v1", "v2"] {
        Mock::given(method("GET"))
            .and(path(format!("/projects/g%2Fp/repository/tags/{tag}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"commit": {"id": SHA}}))
                    .set_delay(DELAY),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let client = GitLabClient::new(server.uri(), None, false);
    let ctx = Ctx::new(Default::default(), 0, ResolverEnv::default())
        .with_gitlab("gitlab.com", Ok(client));
    let (a, b) = tokio::join!(
        ctx.gitlab_resolve(Some("gitlab.com"), "g/p", "v1"),
        ctx.gitlab_resolve(Some("gitlab.com"), "g/p", "v1")
    );
    assert_eq!(a, Ok(SHA.to_string()));
    assert_eq!(b, Ok(SHA.to_string()));
    assert_eq!(
        ctx.gitlab_resolve(Some("gitlab.com"), "g/p", "v2").await,
        Ok(SHA.to_string())
    );
}

/// Characterisation: different tags are different keys.
#[tokio::test]
async fn distinct_keys_are_not_shared() {
    let server = MockServer::start().await;
    mount_digest(&server, "3.20", 200, DIGEST).await;
    mount_digest(&server, "3.21", 200, OTHER).await;
    let ctx = oci_ctx(&server);
    let a = ImageRef::parse("alpine:3.20").unwrap();
    let b = ImageRef::parse("alpine:3.21").unwrap();
    let (da, db) = tokio::join!(ctx.oci_digest(&a), ctx.oci_digest(&b));
    assert_eq!(da, Ok(DIGEST.to_string()));
    assert_eq!(db, Ok(OTHER.to_string()));
}
