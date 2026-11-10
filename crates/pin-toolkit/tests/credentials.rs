//! D31 + D37 through the binary path (spec §7.3, §9 M5d gate, P-token):
//! env map → `fold` → `ResolverEnv::from_vars` → `auth_for` → the wire.
//! A = `gitlab.example.com`, B = `gitlab.com`, two wiremock servers reached
//! through `tune`. The include resolves on the primary host, the component
//! on its own host. Mocks that must never match are mounted first with
//! `expect(0)`; expectations are verified when the servers drop.
#![cfg(feature = "gitlab-ci")]

mod support;

use std::collections::BTreeMap;

use serde_json::json;
use support::{repo, run_tuned, write};
use wiremock::matchers::{header, header_exists, method, path_regex, query_param};
use wiremock::{Mock, MockBuilder, MockServer, ResponseTemplate};

const A: &str = "gitlab.example.com";
const B: &str = "gitlab.com";
const SHA: &str = "e3262fdd0914fa823210cdb79a8c421e2cef79d8";
/// The branch lookups of `g/p@main` (include) and `g/q@main` (component)
/// after the tag lookups (wiremock's 404): a job token uses the list
/// endpoint (`?search=main`, exact name), every other client the
/// single-branch endpoint (`pin_core::gitlab`).
const SINGLE: &str = r"^/projects/g%2F[pq]/repository/branches/main$";
const LIST: &str = r"^/projects/g%2F[pq]/repository/branches$";

fn branch(job: bool) -> MockBuilder {
    if job {
        Mock::given(method("GET"))
            .and(path_regex(LIST))
            .and(query_param("search", "main"))
    } else {
        Mock::given(method("GET")).and(path_regex(SINGLE))
    }
}

fn ok(job: bool) -> ResponseTemplate {
    let commit = json!({"id": SHA});
    ResponseTemplate::new(200).set_body_json(if job {
        json!([{"name": "main", "commit": commit}])
    } else {
        json!({"commit": commit})
    })
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

/// Branch lookups at `server` succeed only with `name: value`, at least once.
async fn require(server: &MockServer, name: &'static str, value: &'static str) {
    let job = name == "JOB-TOKEN";
    branch(job)
        .and(header(name, value))
        .respond_with(ok(job))
        .expect(1..)
        .mount(server)
        .await;
}

/// Anonymous branch lookups at `server` succeed.
async fn allow(server: &MockServer) {
    branch(false).respond_with(ok(false)).mount(server).await;
}

/// A repo whose `.gitlab-ci.yml` has the include (primary) and a component
/// on `component_host`, `gitlab-hosts = [extra]`; runs `pin-toolkit {args}`
/// with `vars`, GitLab A and B on the mock servers; asserts both rows
/// pinned and exit 0.
async fn pin(
    name: &str,
    component_host: &str,
    extra: &str,
    args: &[&str],
    vars: &[(&str, &str)],
    a: &MockServer,
    b: &MockServer,
) {
    let root = repo(name);
    write(
        &root,
        ".pin-toolkit.toml",
        &format!("version = 1\n[plugins.gitlab-ci]\ngitlab-hosts = [\"{extra}\"]\n"),
    );
    write(
        &root,
        ".gitlab-ci.yml",
        &format!(
            "include:\n  - project: g/p\n    ref: main\ncomponent: {component_host}/g/q/c@main\n"
        ),
    );
    let argv: Vec<&str> = std::iter::once("pin-toolkit")
        .chain(args.iter().copied())
        .collect();
    let bases = BTreeMap::from([(A.to_string(), a.uri()), (B.to_string(), b.uri())]);
    let out = run_tuned(&root, &argv, vars, |env| env.base_urls.gitlab = bases).await;
    assert_eq!(
        out.code, 0,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    assert!(
        out.stdout
            .ends_with("pinned: 2, ok: 0, updated: 0, skipped: 0, unresolved: 0\n"),
        "both rows pinned: {}",
        out.stdout
    );
}

#[tokio::test]
async fn ci_job_token_never_sent_to_another_host() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    require(&a, "JOB-TOKEN", "jt").await;
    forbid(&b, "JOB-TOKEN").await;
    allow(&b).await;
    let vars = [("CI_JOB_TOKEN", "jt"), ("CI_SERVER_HOST", A)];
    // Primary B (≠ the CI server A); the component lives on A.
    pin("cred-job", A, A, &["--gitlab-host", B], &vars, &a, &b).await;
}

#[tokio::test]
async fn gitlab_token_only_to_primary() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    require(&a, "PRIVATE-TOKEN", "pt").await;
    forbid(&b, "PRIVATE-TOKEN").await;
    allow(&b).await;
    let vars = [("GITLAB_TOKEN", "pt")];
    // Primary A; the component lives on B.
    pin("cred-private", B, B, &["--gitlab-host", A], &vars, &a, &b).await;
}

#[tokio::test]
async fn ci_server_host_is_default_primary() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    require(&a, "JOB-TOKEN", "jt").await;
    forbid(&b, "JOB-TOKEN").await;
    allow(&b).await;
    let vars = [("CI_SERVER_HOST", A), ("CI_JOB_TOKEN", "jt")];
    // No --gitlab-host / PIN_TOOLKIT_GITLAB_HOST: the primary is A (PR-M10);
    // the include goes there, the component to B, anonymously.
    pin("cred-default", B, B, &[], &vars, &a, &b).await;
}

/// M5d final M1: a `CI_SERVER_HOST` with userinfo would be the default
/// primary and the job token's host, and the request would go to the
/// userinfo-stripped host with both tokens. It is fatal (exit 2, stdout
/// empty) before any request, with or without an explicit primary; no
/// fallback to `gitlab.com`. The bad host's API base is a mock, so a
/// request would be seen.
#[tokio::test]
async fn invalid_ci_server_host_is_fatal_before_any_request() {
    const BAD: &str = "gitlab.com@127.0.0.1:9";
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    for server in [&a, &b] {
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(server)
            .await;
    }
    let root = repo("cred-bad-server-host");
    write(
        &root,
        ".gitlab-ci.yml",
        "include:\n  - project: g/p\n    ref: main\n",
    );
    let vars = [
        ("CI_SERVER_HOST", BAD),
        ("CI_JOB_TOKEN", "jt"),
        ("GITLAB_TOKEN", "pt"),
    ];
    for extra in [&[][..], &["--gitlab-host", B][..]] {
        let argv: Vec<&str> = std::iter::once("pin-toolkit")
            .chain(extra.iter().copied())
            .collect();
        let bases = BTreeMap::from([(BAD.to_string(), a.uri()), (B.to_string(), b.uri())]);
        let out = run_tuned(&root, &argv, &vars, |env| env.base_urls.gitlab = bases).await;
        assert_eq!(out.code, 2, "{argv:?} stderr: {}", out.stderr);
        assert_eq!(out.stdout, "", "{argv:?}");
        assert_eq!(
            out.stderr,
            format!("pin-toolkit: CI_SERVER_HOST: invalid GitLab host `{BAD}`\n"),
            "{argv:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.join(".gitlab-ci.yml")).unwrap(),
        "include:\n  - project: g/p\n    ref: main\n",
        "file untouched"
    );
}
