use pin_ci::image_ref::ImageRef;
use pin_ci::registry::{Creds, RegError, RegistryClient};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

// wiremock 0.6.5 API adaptation (brief was written against a hypothetical API):
//  - `.and_path(p)` -> `.and(path(p))`
//  - `query(k, v)` -> `query_param(k, v)`
//  - `header_exists("authorization", false)` (no such 2-arg form in 0.6.5) -> the
//    NoAuthorization matcher below, same semantics: matches only when the
//    Authorization header is ABSENT.
struct NoAuthorization;

impl Match for NoAuthorization {
    fn matches(&self, request: &Request) -> bool {
        !request.headers.contains_key("authorization")
    }
}

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn client(server: &MockServer, creds: Creds) -> RegistryClient {
    RegistryClient::new(
        creds,
        std::collections::HashMap::from([("docker.io".to_string(), server.uri())]),
    )
}

fn alpine() -> ImageRef {
    ImageRef::parse("alpine:3.20").unwrap()
}

fn tempfile_dir() -> std::path::PathBuf {
    let home = std::env::temp_dir().join(format!("pin-ci-creds-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    home
}

async fn mount_alpine_digest(server: &MockServer, code: u16, with_auth: bool) {
    let mut m = Mock::given(method("HEAD")).and(path("/v2/library/alpine/manifests/3.20"));
    if with_auth {
        m = m.and(header("authorization", "Bearer tok123"));
    } else {
        m = m.and(NoAuthorization);
    }
    m.respond_with(ResponseTemplate::new(code).insert_header("docker-content-digest", DIGEST))
        .mount(server)
        .await;
}

#[tokio::test]
async fn anon_digest_from_header() {
    let server = MockServer::start().await;
    mount_alpine_digest(&server, 200, false).await;
    let c = client(&server, Creds::none());
    assert_eq!(c.digest(&alpine()).await.unwrap(), DIGEST);
}

#[tokio::test]
async fn bearer_flow_on_401() {
    let server = MockServer::start().await;
    mount_alpine_digest(&server, 200, true).await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .and(NoAuthorization)
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            format!("Bearer realm=\"{}/token\",service=\"registry.mock\",scope=\"repository:library/alpine:pull\"", server.uri()),
        ))
        .mount(&server).await;
    let _token_calls = Mock::given(method("GET"))
        .and(path("/token"))
        .and(query_param("service", "registry.mock"))
        .and(query_param("scope", "repository:library/alpine:pull"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "tok123"})),
        )
        .expect(1) // wiremock 0.6: expect() is on Mock, after respond_with()
        .mount(&server)
        .await;
    let c = client(&server, Creds::none());
    assert_eq!(c.digest(&alpine()).await.unwrap(), DIGEST);
}

#[tokio::test]
async fn creds_send_basic_auth_to_token_endpoint() {
    let server = MockServer::start().await;
    mount_alpine_digest(&server, 200, true).await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .and(NoAuthorization)
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            format!("Bearer realm=\"{}/token\",service=\"registry.mock\",scope=\"repository:library/alpine:pull\"", server.uri()),
        ))
        .mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/token"))
        .and(header("authorization", "Basic dXNlcjpwYXNz")) // base64("user:pass")
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "tok123"})),
        )
        .expect(1) // wiremock 0.6: expect() is on Mock, after respond_with()
        .mount(&server)
        .await;
    let c = client(
        &server,
        Creds {
            dockerhub: Some(("user".into(), "pass".into())),
        },
    );
    assert_eq!(c.digest(&alpine()).await.unwrap(), DIGEST);
}

#[tokio::test]
async fn token_is_cached_per_scope() {
    let server = MockServer::start().await;
    mount_alpine_digest(&server, 200, true).await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .and(NoAuthorization)
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            format!("Bearer realm=\"{}/token\",service=\"registry.mock\",scope=\"repository:library/alpine:pull\"", server.uri()),
        ))
        .expect(1) // wiremock 0.6: expect() is on Mock, after respond_with()
        .mount(&server).await;
    let _tok = Mock::given(method("GET"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "tok123"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server, Creds::none());
    let a = ImageRef::parse("alpine:3.20").unwrap();
    let b = ImageRef::parse("alpine:3.21").unwrap(); // different tag, same scope
    // (brief omission) the anonymous HEAD for the second tag must also 401 so the
    // client takes the cached-token path; wiremock answers unmatched requests with 404.
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.21"))
        .and(NoAuthorization)
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            format!("Bearer realm=\"{}/token\",service=\"registry.mock\",scope=\"repository:library/alpine:pull\"", server.uri()),
        ))
        .mount(&server).await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.21"))
        .and(header("authorization", "Bearer tok123"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", DIGEST))
        .mount(&server)
        .await;
    assert_eq!(c.digest(&a).await.unwrap(), DIGEST);
    assert_eq!(c.digest(&b).await.unwrap(), DIGEST); // must reuse tok123, no 2nd /token
}

#[tokio::test]
async fn falls_back_to_body_sha256_when_header_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("hello", "application/octet-stream"))
        .mount(&server)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200)) // no docker-content-digest header
        .mount(&server)
        .await;
    let c = client(&server, Creds::none());
    assert_eq!(
        c.digest(&alpine()).await.unwrap(),
        "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[tokio::test]
async fn list_tags_single_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/library/alpine/tags/list"))
        .and(query_param("n", "1000"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"tags": ["a", "b"]})),
        )
        .mount(&server)
        .await;
    let c = client(&server, Creds::none());
    assert_eq!(c.list_tags(&alpine()).await.unwrap(), vec!["a", "b"]);
}

#[tokio::test]
async fn list_tags_follows_link_next() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/library/alpine/tags/list"))
        .and(query_param("n", "1000"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"tags": ["a", "b"]}))
                .insert_header(
                    "link",
                    format!(
                        r#"<{}/v2/library/alpine/tags/list?last=b>; rel="next""#,
                        server.uri()
                    ),
                ),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/library/alpine/tags/list"))
        .and(query_param("last", "b"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"tags": ["c"]})))
        .mount(&server)
        .await;
    let c = client(&server, Creds::none());
    assert_eq!(c.list_tags(&alpine()).await.unwrap(), vec!["a", "b", "c"]);
}

#[tokio::test]
async fn list_tags_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/library/mystery/tags/list"))
        .and(query_param("n", "1000"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let c = client(&server, Creds::none());
    let err = c
        .list_tags(&ImageRef::parse("mystery:1.0").unwrap())
        .await
        .unwrap_err();
    match err {
        RegError::Unresolved(msg) => assert!(msg.contains("library/mystery not found")),
        other => panic!("expected Unresolved, got {other:?}"),
    }
}

#[tokio::test]
async fn list_tags_bearer_flow() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/library/alpine/tags/list"))
        .and(query_param("n", "1000"))
        .and(NoAuthorization)
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            format!(
                "Bearer realm=\"{}/token\",service=\"registry.mock\",scope=\"repository:library/alpine:pull\"",
                server.uri()
            ),
        ))
        .mount(&server)
        .await;
    let _token = Mock::given(method("GET"))
        .and(path("/token"))
        .and(query_param("service", "registry.mock"))
        .and(query_param("scope", "repository:library/alpine:pull"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "tok123"})),
        )
        .expect(1) // wiremock 0.6: expect() is on Mock, after respond_with()
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/library/alpine/tags/list"))
        .and(query_param("n", "1000"))
        .and(header("authorization", "Bearer tok123"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"tags": ["a", "b"]})),
        )
        .mount(&server)
        .await;
    let c = client(&server, Creds::none());
    assert_eq!(c.list_tags(&alpine()).await.unwrap(), vec!["a", "b"]);
}

#[tokio::test]
async fn list_tags_caches_per_repo() {
    let server = MockServer::start().await;
    let _tags = Mock::given(method("GET"))
        .and(path("/v2/library/alpine/tags/list"))
        .and(query_param("n", "1000"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"tags": ["a", "b"]})),
        )
        .expect(1) // second list_tags call must be served from the per-repo cache
        .mount(&server)
        .await;
    let c = client(&server, Creds::none());
    assert_eq!(c.list_tags(&alpine()).await.unwrap(), vec!["a", "b"]);
    assert_eq!(c.list_tags(&alpine()).await.unwrap(), vec!["a", "b"]);
}

#[tokio::test]
async fn not_found_is_unresolved() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/mystery/manifests/1.0"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let c = client(&server, Creds::none());
    let err = c
        .digest(&ImageRef::parse("mystery:1.0").unwrap())
        .await
        .unwrap_err();
    match err {
        RegError::Unresolved(msg) => assert!(msg.contains("library/mystery")),
        other => panic!("expected Unresolved, got {other:?}"),
    }
}

#[test]
fn creds_env_precedence_and_config_json() {
    let home = tempfile_dir(); // helper: std::env::temp_dir().join("pin-ci-creds-test-..."), created fresh
    std::fs::create_dir_all(home.join(".docker")).unwrap();
    std::fs::write(
        home.join(".docker/config.json"),
        r#"{"auths":{"https://index.docker.io/v1/":{"username":"u","password":"p"}}}"#,
    )
    .unwrap();
    let c = Creds::load_from(None, None, Some(&home));
    assert_eq!(
        c.dockerhub.as_ref().unwrap(),
        &("u".to_string(), "p".to_string())
    );
    let c = Creds::load_from(Some("eu".into()), Some("ep".into()), Some(&home));
    assert_eq!(
        c.dockerhub.as_ref().unwrap(),
        &("eu".to_string(), "ep".to_string())
    );
    std::fs::remove_dir_all(&home).ok();
}
