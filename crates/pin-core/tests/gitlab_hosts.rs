//! D31 + D37 on the wire (T7): with two GitLab hosts in one run,
//! `CI_JOB_TOKEN` reaches only `CI_SERVER_HOST`, `GITLAB_TOKEN` only the
//! primary host and a global `[hosts]` token only its own host (spec §7.3);
//! a failure on a non-primary host without a credential reads `not trusted
//! for credentials` (D45). A = `gitlab.example.com`, B = `gitlab.com`, both
//! wiremock servers reached through `base_urls.gitlab` (feature
//! `test-support`).
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

fn env(
    vars: &[(&str, &str)],
    primary: &str,
    hosts: &[&str],
    a: &MockServer,
    b: &MockServer,
) -> ResolverEnv {
    let vars: BTreeMap<String, String> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let hosts: BTreeSet<String> = hosts.iter().map(|h| h.to_string()).collect();
    let mut env = ResolverEnv::from_vars(&vars, Some(primary.to_string()), hosts).unwrap();
    env.base_urls.gitlab = BTreeMap::from([(A.to_string(), a.uri()), (B.to_string(), b.uri())]);
    env
}

fn ctx(
    vars: &[(&str, &str)],
    primary: &str,
    hosts: &[&str],
    a: &MockServer,
    b: &MockServer,
) -> Ctx {
    Ctx::new(Default::default(), 0, env(vars, primary, hosts, a, b))
}

/// [`ctx`] with `bound` as the global config's `[hosts]` tokens.
fn ctx_bound(
    vars: &[(&str, &str)],
    primary: &str,
    hosts: &[&str],
    bound: &[(&str, &str)],
    a: &MockServer,
    b: &MockServer,
) -> Ctx {
    let mut env = env(vars, primary, hosts, a, b);
    env.gitlab.bound = bound
        .iter()
        .map(|(h, t)| (h.to_string(), t.to_string()))
        .collect();
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

/// Spec §7.3: a `[hosts]` token reaches only its own host (A), never the
/// primary (B).
#[tokio::test]
async fn bound_token_only_reaches_bound_host() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("GET"))
        .and(path(TAG))
        .and(header("PRIVATE-TOKEN", "bt"))
        .respond_with(ok())
        .expect(1)
        .mount(&a)
        .await;
    forbid(&b, "PRIVATE-TOKEN").await;
    allow(&b).await;
    let ctx = ctx_bound(&[], B, &[A], &[(A, "bt")], &a, &b);
    resolve_both(&ctx).await;
}

/// The env bindings win over `[hosts]`: `GITLAB_TOKEN` to the primary (A),
/// `CI_JOB_TOKEN` to `CI_SERVER_HOST` (B); neither bound token is sent.
#[tokio::test]
async fn bound_token_loses_to_env_bindings() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("GET"))
        .and(header("PRIVATE-TOKEN", "bt-a"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&a)
        .await;
    forbid(&b, "PRIVATE-TOKEN").await;
    Mock::given(method("GET"))
        .and(path(TAG))
        .and(header("PRIVATE-TOKEN", "pt"))
        .respond_with(ok())
        .expect(1)
        .mount(&a)
        .await;
    Mock::given(method("GET"))
        .and(path(TAG))
        .and(header("JOB-TOKEN", "jt"))
        .respond_with(ok())
        .expect(1)
        .mount(&b)
        .await;
    let vars = [
        ("GITLAB_TOKEN", "pt"),
        ("CI_JOB_TOKEN", "jt"),
        ("CI_SERVER_HOST", B),
    ];
    let bound = [(A, "bt-a"), (B, "bt-b")];
    let ctx = ctx_bound(&vars, A, &[B], &bound, &a, &b);
    resolve_both(&ctx).await;
}

/// Every GET on `server` answers `code`.
async fn fail_all(server: &MockServer, code: u16) {
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(code))
        .mount(server)
        .await;
}

const UNAUTHORIZED: &str = "gitlab api: 401 unauthorized (check GITLAB_TOKEN/CI_JOB_TOKEN)";
const NOT_FOUND: &str = "g/p:v1 not found (no tag or branch)";

/// D45: a failed lookup on a non-primary host without a credential names
/// the global config that could bind one (here the front-end's path).
#[tokio::test]
async fn unbound_host_failure_401_is_not_trusted() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    fail_all(&a, 401).await;
    allow(&b).await;
    let mut env = env(&[("GITLAB_TOKEN", "pt")], B, &[A], &a, &b);
    env.gitlab.global_config = Some("/xdg/pin-toolkit/config.toml".into());
    let ctx = Ctx::new(Default::default(), 0, env);
    assert_eq!(
        ctx.gitlab_resolve(Some(A), "g/p", "v1").await,
        Err(format!(
            "{A}: not trusted for credentials — bind it in /xdg/pin-toolkit/config.toml"
        ))
    );
    assert_eq!(ctx.gitlab_resolve(None, "g/p", "v1").await, Ok(SHA.into()));
}

/// A 404 (no tag, no branch) too; no path passed = the standard one. A
/// `CI_JOB_TOKEN` without `CI_SERVER_HOST` binds nothing (D37).
#[tokio::test]
async fn unbound_host_failure_404_names_the_default_path() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    let ctx = ctx(&[("CI_JOB_TOKEN", "jt")], B, &[A], &a, &b);
    assert_eq!(
        ctx.gitlab_resolve(Some(A), "g/p", "v1").await,
        Err(format!(
            "{A}: not trusted for credentials — bind it in ~/.config/pin-toolkit/config.toml"
        ))
    );
}

/// Review L2: `CI_SERVER_HOST` without `CI_JOB_TOKEN` binds nothing, so a
/// failure there is D45 too.
#[tokio::test]
async fn unbound_host_failure_ci_server_host_without_job_token() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    fail_all(&a, 401).await;
    let vars = [("CI_SERVER_HOST", A), ("GITLAB_TOKEN", "pt")];
    let ctx = ctx(&vars, B, &[A], &a, &b);
    assert_eq!(
        ctx.gitlab_resolve(Some(A), "g/p", "v1").await,
        Err(format!(
            "{A}: not trusted for credentials — bind it in ~/.config/pin-toolkit/config.toml"
        ))
    );
}

/// The primary host keeps the client's texts, with or without a token.
#[tokio::test]
async fn unbound_host_failure_on_primary_keeps_message() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    fail_all(&a, 401).await;
    for vars in [&[][..], &[("GITLAB_TOKEN", "pt")][..]] {
        let c = ctx(vars, A, &[B], &a, &b);
        assert_eq!(
            c.gitlab_resolve(None, "g/p", "v1").await,
            Err(UNAUTHORIZED.into())
        );
        assert_eq!(
            c.gitlab_resolve(Some(A), "g/p", "v1").await,
            Err(UNAUTHORIZED.into())
        );
        let c = ctx(vars, B, &[A], &a, &b);
        assert_eq!(
            c.gitlab_resolve(None, "g/p", "v1").await,
            Err(NOT_FOUND.into())
        );
    }
}

/// A non-primary host with a credential (a `[hosts]` token, or
/// `CI_JOB_TOKEN` as `CI_SERVER_HOST`) keeps the client's texts.
#[tokio::test]
async fn unbound_host_failure_bound_host_keeps_message() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    fail_all(&a, 401).await;
    let c = ctx_bound(&[], B, &[A], &[(A, "bt")], &a, &b);
    assert_eq!(
        c.gitlab_resolve(Some(A), "g/p", "v1").await,
        Err(UNAUTHORIZED.into())
    );
    let c = ctx_bound(&[], A, &[B], &[(B, "bt")], &a, &b);
    assert_eq!(
        c.gitlab_resolve(Some(B), "g/p", "v1").await,
        Err(NOT_FOUND.into())
    );
    let vars = [("CI_JOB_TOKEN", "jt"), ("CI_SERVER_HOST", B)];
    let c = ctx(&vars, A, &[B], &a, &b);
    assert_eq!(
        c.gitlab_resolve(Some(B), "g/p", "v1").await,
        Err(NOT_FOUND.into())
    );
}
