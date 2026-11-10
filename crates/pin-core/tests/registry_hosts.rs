//! M4a Task 3 — challenge-driven registry auth against wiremock:
//! GHCR bearer-PAT, ECR-public anonymous, GCP identitytoken, JFrog Basic
//! challenge, no-credentials fail-loud paths, and the unchanged
//! no-challenge 401 error.

use pin_core::oci::image_ref::ImageRef;
use pin_core::oci::registry::{Creds, HostCreds, RegistryClient};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

// M1 lesson: wiremock 0.6.5 has no 2-arg header matcher — the NoAuthorization
// matcher below matches only when the Authorization header is ABSENT.
struct NoAuthorization;

impl Match for NoAuthorization {
    fn matches(&self, request: &Request) -> bool {
        !request.headers.contains_key("authorization")
    }
}

const D1: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const D2: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const D3: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
const D4: &str = "sha256:4444444444444444444444444444444444444444444444444444444444444444";

fn client_for(server: &MockServer, host: &str, creds: Creds) -> RegistryClient {
    RegistryClient::new(
        creds,
        std::collections::HashMap::from([(host.to_string(), server.uri())]),
    )
}

/// A fresh temp home dir with a `.docker/config.json` (M1 test pattern).
fn home_with_config(json: &str) -> std::path::PathBuf {
    let home = std::env::temp_dir().join(format!(
        "pin-ci-hosts-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(home.join(".docker")).unwrap();
    std::fs::write(home.join(".docker").join("config.json"), json).unwrap();
    home
}

#[tokio::test]
async fn ghcr_bearer_pat_flow() {
    let server = MockServer::start().await;
    let challenge = format!(
        r#"Bearer realm="{}/token",service="ghcr.io",scope="repository:org/img:pull""#,
        server.uri()
    );
    // Anonymous probe → 401 + ghcr-style challenge.
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/1.0"))
        .and(NoAuthorization)
        .respond_with(ResponseTemplate::new(401).insert_header("www-authenticate", &challenge))
        .mount(&server)
        .await;
    // Token request must carry the PAT as Bearer.
    Mock::given(method("GET"))
        .and(path("/token"))
        .and(header("authorization", "Bearer pat-123"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "token": "ghcr-tok" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/1.0"))
        .and(header("authorization", "Bearer ghcr-tok"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&server)
        .await;
    let c = client_for(
        &server,
        "ghcr.io",
        Creds::none().with_host(
            "ghcr.io",
            HostCreds::Bearer {
                token: "pat-123".into(),
            },
        ),
    );
    let r = ImageRef::parse("ghcr.io/org/img:1.0").unwrap();
    assert_eq!(c.digest(&r).await.unwrap(), D1);
}

#[tokio::test]
async fn ecr_public_anon_flow() {
    let server = MockServer::start().await;
    // ECR public: realm with trailing slash, service public.ecr.aws, scope "aws".
    let challenge = format!(
        r#"Bearer realm="{}/token/",service="public.ecr.aws",scope="aws""#,
        server.uri()
    );
    Mock::given(method("HEAD"))
        .and(path("/v2/nginx/nginx/manifests/latest"))
        .and(NoAuthorization)
        .respond_with(ResponseTemplate::new(401).insert_header("www-authenticate", &challenge))
        .mount(&server)
        .await;
    // Anonymous token request (NO Authorization header).
    Mock::given(method("GET"))
        .and(path("/token/"))
        .and(NoAuthorization)
        .and(query_param("service", "public.ecr.aws"))
        .and(query_param("scope", "aws"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "token": "ecr-anon" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/nginx/nginx/manifests/latest"))
        .and(header("authorization", "Bearer ecr-anon"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D2))
        .mount(&server)
        .await;
    let c = client_for(&server, "public.ecr.aws", Creds::none());
    let r = ImageRef::parse("public.ecr.aws/nginx/nginx:latest").unwrap();
    assert_eq!(c.digest(&r).await.unwrap(), D2);
}

#[tokio::test]
async fn gcp_identitytoken_flow() {
    let home =
        home_with_config(r#"{"auths":{"us-docker.pkg.dev":{"identitytoken":"ya29.gcp-token"}}}"#);
    let server = MockServer::start().await;
    let challenge = format!(
        r#"Bearer realm="{}/token",service="us-docker.pkg.dev",scope="repository:myproj/img:pull""#,
        server.uri()
    );
    Mock::given(method("HEAD"))
        .and(path("/v2/myproj/img/manifests/1"))
        .and(NoAuthorization)
        .respond_with(ResponseTemplate::new(401).insert_header("www-authenticate", &challenge))
        .mount(&server)
        .await;
    // The identitytoken (gcloud) goes on the token request as Bearer.
    Mock::given(method("GET"))
        .and(path("/token"))
        .and(header("authorization", "Bearer ya29.gcp-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "token": "gcp-access" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/myproj/img/manifests/1"))
        .and(header("authorization", "Bearer gcp-access"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D3))
        .mount(&server)
        .await;
    let creds = Creds::load_from(Some(&home));
    assert_eq!(
        creds.for_host("us-docker.pkg.dev"),
        Some(&HostCreds::Bearer {
            token: "ya29.gcp-token".into()
        })
    );
    let c = client_for(&server, "us-docker.pkg.dev", creds);
    let r = ImageRef::parse("us-docker.pkg.dev/myproj/img:1").unwrap();
    assert_eq!(c.digest(&r).await.unwrap(), D3);
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn jfrog_basic_challenge() {
    let home = home_with_config(
        r#"{"auths":{"artifactory.example.com":{"username":"jenkins","password":"s3cret"}}}"#,
    );
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/v2/docker/lib/alpine/manifests/3"))
        .and(NoAuthorization)
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("www-authenticate", r#"Basic realm="Artifactory Realm""#),
        )
        .mount(&server)
        .await;
    // No token round-trip: the manifest is retried directly with Basic.
    Mock::given(method("HEAD"))
        .and(path("/v2/docker/lib/alpine/manifests/3"))
        .and(header("authorization", "Basic amVua2luczpzM2NyZXQ="))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D4))
        .mount(&server)
        .await;
    let creds = Creds::load_from(Some(&home));
    let c = client_for(&server, "artifactory.example.com", creds);
    let r = ImageRef::parse("artifactory.example.com/docker/lib/alpine:3").unwrap();
    assert_eq!(c.digest(&r).await.unwrap(), D4);
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn basic_challenge_no_creds() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/v2/a/b/manifests/1"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("www-authenticate", r#"Basic realm="Artifactory Realm""#),
        )
        .mount(&server)
        .await;
    let c = client_for(&server, "artifactory.example.com", Creds::none());
    let r = ImageRef::parse("artifactory.example.com/a/b:1").unwrap();
    let err = c.digest(&r).await.unwrap_err().to_string();
    assert!(
        err.contains("artifactory.example.com: registry requires basic login")
            && err.contains("docker login artifactory.example.com"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn token_denied_no_creds() {
    // ghcr: token endpoint denies the anonymous request → hint names GITHUB_TOKEN.
    let server = MockServer::start().await;
    let challenge = format!(
        r#"Bearer realm="{}/token",service="ghcr.io",scope="repository:org/img:pull""#,
        server.uri()
    );
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/1.0"))
        .respond_with(ResponseTemplate::new(401).insert_header("www-authenticate", &challenge))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(serde_json::json!({ "errors": [{ "code": "DENIED" }] })),
        )
        .mount(&server)
        .await;
    let c = client_for(&server, "ghcr.io", Creds::none());
    let r = ImageRef::parse("ghcr.io/org/img:1.0").unwrap();
    let err = c.digest(&r).await.unwrap_err().to_string();
    assert!(
        err.contains("token denied") && err.contains("GITHUB_TOKEN"),
        "unexpected error: {err}"
    );

    // Generic host: the hint is the plain docker-login one.
    let server = MockServer::start().await;
    let challenge = format!(
        r#"Bearer realm="{}/token",service="myreg.example.com",scope="repository:a/b:pull""#,
        server.uri()
    );
    Mock::given(method("HEAD"))
        .and(path("/v2/a/b/manifests/1"))
        .respond_with(ResponseTemplate::new(401).insert_header("www-authenticate", &challenge))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let c = client_for(&server, "myreg.example.com", Creds::none());
    let r = ImageRef::parse("myreg.example.com/a/b:1").unwrap();
    let err = c.digest(&r).await.unwrap_err().to_string();
    assert!(
        err.contains("token denied") && err.contains("docker login myreg.example.com"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn no_challenge_401_unchanged() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/v2/a/b/manifests/1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let c = client_for(&server, "myreg.example.com", Creds::none());
    let r = ImageRef::parse("myreg.example.com/a/b:1").unwrap();
    let err = c.digest(&r).await.unwrap_err().to_string();
    // M1 error text, byte-identical.
    assert!(
        err.contains("401 without WWW-Authenticate at /v2/a/b/manifests/1"),
        "unexpected error: {err}"
    );
}
