//! D31 + D37 on the wire (T7): with two GitLab hosts in one run,
//! `CI_JOB_TOKEN` reaches only `CI_SERVER_HOST` and `GITLAB_TOKEN` only the
//! primary host. A = `gitlab.example.com`, B = `gitlab.com`, both wiremock
//! servers reached through `base_urls.gitlab` (feature `test-support`).
//! Mocks that must never match are mounted first with `expect(0)`; the
//! expectations are verified when the servers drop.

use std::collections::{BTreeMap, BTreeSet};

use pin_core::run::{Ctx, ResolverEnv};
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const A: &str = "gitlab.example.com";
const B: &str = "gitlab.com";
const SHA: &str = "e3262fdd0914fa823210cdb79a8c421e2cef79d8";
const TAG: &str = "/projects/g%2Fp/repository/tags/v1";

fn ok() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({"commit": {"id": SHA}}))
}

/// A request carrying `name` must never arrive at `server`.
async fn forbid(server: &MockServer, name: &'static str) {
    Mock::given(method("GET"))
        .and(header_exists(name))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(server)
        .await;
}

/// Any other tag lookup succeeds.
async fn allow(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(TAG))
        .respond_with(ok())
        .mount(server)
        .await;
}

fn ctx(
    vars: &[(&str, &str)],
    primary: &str,
    hosts: &[&str],
    a: &MockServer,
    b: &MockServer,
) -> Ctx {
    let vars: BTreeMap<String, String> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let hosts: BTreeSet<String> = hosts.iter().map(|h| h.to_string()).collect();
    let mut env = ResolverEnv::from_vars(&vars, Some(primary.to_string()), hosts).unwrap();
    env.base_urls.gitlab = BTreeMap::from([(A.to_string(), a.uri()), (B.to_string(), b.uri())]);
    Ctx::new(Default::default(), 0, env)
}

async fn resolve_both(ctx: &Ctx) {
    for host in [A, B] {
        assert_eq!(
            ctx.gitlab_resolve(Some(host), "g/p", "v1").await,
            Ok(SHA.to_string()),
            "{host}"
        );
    }
}

#[tokio::test]
async fn job_token_only_reaches_ci_server_host() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("GET"))
        .and(path(TAG))
        .and(header("JOB-TOKEN", "jt"))
        .respond_with(ok())
        .expect(1)
        .mount(&a)
        .await;
    forbid(&b, "JOB-TOKEN").await;
    allow(&b).await;
    let ctx = ctx(
        &[("CI_JOB_TOKEN", "jt"), ("CI_SERVER_HOST", A)],
        B,
        &[A],
        &a,
        &b,
    );
    resolve_both(&ctx).await;
}

#[tokio::test]
async fn job_token_without_ci_server_host_is_never_sent() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    for s in [&a, &b] {
        forbid(s, "JOB-TOKEN").await;
        allow(s).await;
    }
    let ctx = ctx(&[("CI_JOB_TOKEN", "jt")], B, &[A], &a, &b);
    resolve_both(&ctx).await;
}

#[tokio::test]
async fn private_token_only_reaches_primary() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("GET"))
        .and(path(TAG))
        .and(header("PRIVATE-TOKEN", "pt"))
        .respond_with(ok())
        .expect(1)
        .mount(&a)
        .await;
    forbid(&b, "PRIVATE-TOKEN").await;
    allow(&b).await;
    let ctx = ctx(&[("GITLAB_TOKEN", "pt")], A, &[B], &a, &b);
    resolve_both(&ctx).await;
}
