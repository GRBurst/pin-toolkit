//! M4b-1 Task 1 — Docker Hub tag push-timestamps client against wiremock.

use pin_ci::image_ref::ImageRef;
use pin_ci::registry::RegistryClient;
use std::collections::HashMap;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Fixture timestamps, `date -ud` verified 2026-11-09:
const TS_2020: i64 = 1577836800; // 2020-01-01T00:00:00Z
const TS_2021: i64 = 1623758400; // 2021-06-15T12:00:00Z
const TS_2019: i64 = 1558341000; // 2019-05-20T08:30:00Z

fn hub_client(server: &MockServer) -> RegistryClient {
    RegistryClient::new(pin_ci::registry::Creds::none(), HashMap::new())
        .with_hub_base(&server.uri())
}

#[tokio::test]
async fn hub_single_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/repositories/library/alpine/tags"))
        .and(query_param("page_size", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "count": 3,
            "next": null,
            "results": [
                { "name": "3.24", "tag_last_pushed": "2020-01-01T00:00:00Z" },
                { "name": "3.20", "last_updated": "2021-06-15T12:00:00Z" },
                { "name": "no-ts" }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let c = hub_client(&server);
    let r = ImageRef::parse("alpine:3.24").unwrap();
    let m = c.tag_timestamps(&r).await.unwrap();
    assert_eq!(m.len(), 2, "no-ts entry must be absent");
    assert_eq!(m.get("3.24"), Some(&TS_2020));
    assert_eq!(m.get("3.20"), Some(&TS_2021));
}

#[tokio::test]
async fn hub_pagination() {
    let server = MockServer::start().await;
    let page2 = format!("{}/page2", server.uri());
    Mock::given(method("GET"))
        .and(path("/v2/repositories/library/alpine/tags"))
        .and(query_param("page_size", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "count": 3,
            "next": page2,
            "results": [
                { "name": "3.24", "tag_last_pushed": "2020-01-01T00:00:00Z" }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "count": 3,
            "next": null,
            "results": [
                { "name": "3.19", "tag_last_pushed": "2019-05-20T08:30:00Z" }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let c = hub_client(&server);
    let r = ImageRef::parse("alpine:3.24").unwrap();
    let m = c.tag_timestamps(&r).await.unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m.get("3.24"), Some(&TS_2020));
    assert_eq!(m.get("3.19"), Some(&TS_2019));
}

#[tokio::test]
async fn hub_tag_last_pushed_wins() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/repositories/library/nginx/tags"))
        .and(query_param("page_size", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "count": 1,
            "next": null,
            "results": [
                {
                    "name": "1.25",
                    "tag_last_pushed": "2020-01-01T00:00:00Z",
                    "last_updated": "2021-06-15T12:00:00Z"
                }
            ]
        })))
        .mount(&server)
        .await;
    let c = hub_client(&server);
    let r = ImageRef::parse("nginx:1.25").unwrap();
    let m = c.tag_timestamps(&r).await.unwrap();
    // tag_last_pushed preferred over last_updated
    assert_eq!(m.get("1.25"), Some(&TS_2020));
}

#[tokio::test]
async fn hub_404_repo_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/repositories/nosuch/repo/tags"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(serde_json::json!({ "message": "Not Found" })),
        )
        .mount(&server)
        .await;
    let c = hub_client(&server);
    let r = ImageRef::parse("nosuch/repo:1").unwrap();
    assert!(c.tag_timestamps(&r).await.unwrap().is_empty());
}

#[tokio::test]
async fn hub_429_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/repositories/library/alpine/tags"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "60"))
        .mount(&server)
        .await;
    let c = hub_client(&server);
    let r = ImageRef::parse("alpine:3.24").unwrap();
    let err = c.tag_timestamps(&r).await.unwrap_err().to_string();
    assert!(err.contains("429"), "unexpected error: {err}");
}

#[tokio::test]
async fn hub_malformed_page_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/repositories/library/alpine/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "count": 0 })))
        .mount(&server)
        .await;
    let c = hub_client(&server);
    let r = ImageRef::parse("alpine:3.24").unwrap();
    let err = c.tag_timestamps(&r).await.unwrap_err().to_string();
    assert!(err.contains("hub api"), "unexpected error: {err}");
}

#[tokio::test]
async fn hub_nonhub_ref_empty() {
    let server = MockServer::start().await;
    // No mocks mounted: ANY request would fail the test (wiremock rejects unmocked).
    let c = hub_client(&server);
    for ref_str in ["ghcr.io/org/img:1.0", "public.ecr.aws/nginx/nginx:latest"] {
        let r = ImageRef::parse(ref_str).unwrap();
        assert!(
            c.tag_timestamps(&r).await.unwrap().is_empty(),
            "{ref_str} must short-circuit to empty without HTTP"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
