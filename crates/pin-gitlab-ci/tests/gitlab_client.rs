//! Wiremock tests for `GitLabClient` (M3 Task 3): tag/branch resolution,
//! URL-encoding, auth headers, the job-token branch-list path, and the
//! per-`(project, ref)` cache. Patterns follow `tests/mock_registry.rs`
//! (wiremock 0.6.5: `.and(path(...))`, `query_param`, `header`, `expect(n)`
//! on `Mock` after `respond_with`; expectations verified on server drop).

use pin_ci::gitlab::{GitLabClient, GitLabError};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SHA: &str = "e3262fdd0914fa823210cdb79a8c421e2cef79d8";
/// A different SHA, so "the right one came back" is meaningful.
const OTHER: &str = "ffffffffffffffffffffffffffffffffffff01";

fn client(server: &MockServer, token: Option<&str>, job_token: bool) -> GitLabClient {
    GitLabClient::new(server.uri(), token.map(str::to_string), job_token)
}

fn commit_body(id: &str) -> serde_json::Value {
    serde_json::json!({"commit": {"id": id}})
}

/// Tag 200 → `commit.id`; the project segment `g/p` must arrive as `g%2Fp`
/// (RFC 3986 encoding of `/`).
#[tokio::test]
async fn tag_200_returns_commit_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/v1.0.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(commit_body(SHA)))
        .mount(&server)
        .await;
    let c = client(&server, None, false);
    assert_eq!(c.resolve("g/p", "v1.0.0").await.unwrap(), SHA);
}

/// Annotated tag: the tag object's own id is in `target`, not `commit.id`;
/// the client must return `commit.id`.
#[tokio::test]
async fn annotated_uses_commit_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/v2.0.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "v2.0.0",
            "target": OTHER,
            "commit": {"id": SHA}
        })))
        .mount(&server)
        .await;
    let c = client(&server, None, false);
    assert_eq!(c.resolve("g/p", "v2.0.0").await.unwrap(), SHA);
}

/// PAT branch: single-branch endpoint, `feature/x` encoded as
/// `feature%2Fx`.
#[tokio::test]
async fn branch_200_returns_commit_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/branches/feature%2Fx"))
        .respond_with(ResponseTemplate::new(200).set_body_json(commit_body(SHA)))
        .mount(&server)
        .await;
    let c = client(&server, Some("pat"), false);
    assert_eq!(c.resolve("g/p", "feature/x").await.unwrap(), SHA);
}

/// Bare name: the tag endpoint is tried first; when it 200s, the branch
/// endpoint must NOT be called.
#[tokio::test]
async fn bare_name_tag_first() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(commit_body(SHA)))
        .expect(1)
        .mount(&server)
        .await;
    let _branch = Mock::given(method("GET"))
        .and(path_regex_branch_any())
        .respond_with(ResponseTemplate::new(200).set_body_json(commit_body(OTHER)))
        .expect(0)
        .mount(&server)
        .await;
    let c = client(&server, None, false);
    assert_eq!(c.resolve("g/p", "main").await.unwrap(), SHA);
}

/// Bare name: tag 404 falls back to the branch endpoint.
#[tokio::test]
async fn bare_name_falls_back_to_branch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/main"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/branches/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(commit_body(SHA)))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server, None, false);
    assert_eq!(c.resolve("g/p", "main").await.unwrap(), SHA);
}

/// Bare name: tag 404 AND branch 404 → `Unresolved` containing the project
/// and the ref.
#[tokio::test]
async fn bare_name_both_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/main"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/branches/main"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let c = client(&server, None, false);
    // `GitLabError` has exactly one variant, so this binding is exhaustive.
    let GitLabError::Unresolved(msg) = c.resolve("g/p", "main").await.unwrap_err();
    assert!(
        msg.contains("g/p"),
        "message must contain the project: {msg}"
    );
    assert!(msg.contains("main"), "message must contain the ref: {msg}");
    assert!(
        msg.contains("not found (no tag or branch)"),
        "message: {msg}"
    );
}

/// `refs/tags/<t>` goes straight to the tag endpoint (no branch call);
/// `refs/heads/<b>` goes straight to the branch endpoint (no tag call).
#[tokio::test]
async fn refs_qualified_routing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/v1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(commit_body(SHA)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/branches/b"))
        .respond_with(ResponseTemplate::new(200).set_body_json(commit_body(OTHER)))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server, None, false);
    assert_eq!(c.resolve("g/p", "refs/tags/v1").await.unwrap(), SHA);
    assert_eq!(c.resolve("g/p", "refs/heads/b").await.unwrap(), OTHER);
}

/// 401 → `Unresolved` with the status and a token hint.
#[tokio::test]
async fn http_401_unresolved() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/v1.0.0"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(serde_json::json!({"message": "401 Unauthorized"})),
        )
        .mount(&server)
        .await;
    let c = client(&server, Some("bad"), false);
    // `GitLabError` has exactly one variant, so this binding is exhaustive.
    let GitLabError::Unresolved(msg) = c.resolve("g/p", "v1.0.0").await.unwrap_err();
    assert!(msg.contains("401"), "message: {msg}");
    assert!(msg.contains("GITLAB_TOKEN"), "message: {msg}");
    assert!(msg.contains("CI_JOB_TOKEN"), "message: {msg}");
}

/// 429 → `Unresolved` with the rate-limited text.
#[tokio::test]
async fn http_429_unresolved() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/v1.0.0"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    let c = client(&server, Some("tok"), false);
    // `GitLabError` has exactly one variant, so this binding is exhaustive.
    let GitLabError::Unresolved(msg) = c.resolve("g/p", "v1.0.0").await.unwrap_err();
    assert!(msg.contains("429"), "message: {msg}");
}

/// Job-token branch resolution MUST use the list endpoint
/// `GET /projects/:id/repository/branches?search=<name>` with the
/// `JOB-TOKEN` header, and match the branch by EXACT name — the near-miss
/// `main-x` in the response must not win.
#[tokio::test]
async fn job_token_branch_via_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/branches"))
        .and(wiremock::matchers::query_param("search", "main"))
        .and(header("job-token", "jtok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"name": "main-x", "commit": {"id": OTHER}},
            {"name": "main", "commit": {"id": SHA}}
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server, Some("jtok"), true);
    assert_eq!(c.resolve("g/p", "refs/heads/main").await.unwrap(), SHA);
}

/// PAT branch resolution uses the single-branch endpoint with the
/// `PRIVATE-TOKEN` header.
#[tokio::test]
async fn pat_branch_single_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/branches/main"))
        .and(header("private-token", "pat123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(commit_body(SHA)))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server, Some("pat123"), false);
    assert_eq!(c.resolve("g/p", "refs/heads/main").await.unwrap(), SHA);
}

/// The per-`(project, ref)` cache: resolving the same (project, ref) twice
/// hits the tag endpoint exactly once.
#[tokio::test]
async fn cache_hit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/v1.0.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(commit_body(SHA)))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server, None, false);
    assert_eq!(c.resolve("g/p", "v1.0.0").await.unwrap(), SHA);
    assert_eq!(c.resolve("g/p", "v1.0.0").await.unwrap(), SHA);
}

/// Matcher helper: any request to the branch endpoint (single or list).
fn path_regex_branch_any() -> wiremock::matchers::PathRegexMatcher {
    wiremock::matchers::path_regex(r"^/projects/g%2Fp/repository/branches")
}
