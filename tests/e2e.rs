use pin_ci::{PinOptions, RegistryClient, exit_code, pin_files};
use std::path::PathBuf;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const D2: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const D3: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
const D4: &str = "sha256:4444444444444444444444444444444444444444444444444444444444444444";
const D5: &str = "sha256:5555555555555555555555555555555555555555555555555555555555555555";
const FIXTURE: &str = include_str!("fixtures/ci-e2e.yml");
const UPDATE_FIXTURE: &str = include_str!("fixtures/ci-update.yml");

async fn mock_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/mystery/manifests/1.0"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&s)
        .await;
    s
}

fn write_fixture(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("ci-e2e.yml");
    std::fs::write(&p, FIXTURE).unwrap();
    p
}

fn write_update_fixture(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("ci-update.yml");
    std::fs::write(&p, UPDATE_FIXTURE).unwrap();
    p
}

fn mock_client(s: &MockServer) -> RegistryClient {
    RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([("docker.io".to_string(), s.uri())]),
    )
}

fn tag_list_mock(repo: &str, tags: &[&str]) -> Mock {
    Mock::given(method("GET"))
        .and(path(format!("/v2/{repo}/tags/list")))
        .and(query_param("n", "1000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"tags": tags})))
}

#[tokio::test]
async fn pins_and_reports_all_states() {
    let s = mock_server().await;
    let client = RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([("docker.io".to_string(), s.uri())]),
    );
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_fixture(&dir);
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: false,
        update_major: false,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    let out = std::fs::read_to_string(&p).unwrap();
    let expected = format!(
        "image: alpine:3.20@{D1}\nghcr-job:\n  image: ghcr.io/org/img:v1\nmissing-job:\n  image: mystery:1.0\ndup:\n  image: alpine:3.20@{D1}\n"
    );
    assert_eq!(out, expected);
    let by_status: std::collections::HashMap<&str, usize> = report
        .results
        .iter()
        .map(|r| (r.status.as_str(), 1))
        .fold(std::collections::HashMap::new(), |mut m, (s, n)| {
            *m.entry(s).or_insert(0) += n;
            m
        });
    assert_eq!(by_status.get("pinned"), Some(&2)); // two alpine lines, same digest
    assert_eq!(by_status.get("skipped"), Some(&1)); // ghcr
    assert_eq!(by_status.get("unresolved"), Some(&1)); // mystery 404
    assert_eq!(exit_code(&report), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn second_run_is_noop() {
    let s = mock_server().await;
    let client = RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([("docker.io".to_string(), s.uri())]),
    );
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_fixture(&dir);
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: false,
        update_major: false,
    };
    let _ = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    let after_first = std::fs::read_to_string(&p).unwrap();
    let report2 = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), after_first);
    assert!(
        report2
            .results
            .iter()
            .all(|r| r.status == "ok" || r.status == "skipped" || r.status == "unresolved")
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dry_run_writes_nothing() {
    let s = mock_server().await;
    let client = RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([("docker.io".to_string(), s.uri())]),
    );
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_fixture(&dir);
    let opts = PinOptions {
        dry_run: true,
        json: false,
        concurrency: 4,
        update: false,
        update_major: false,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), FIXTURE);
    assert_eq!(report.summary.pinned, 2);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn templated_image_is_skipped_not_unresolved() {
    // Shell/YAML-templated image lines are syntactic candidates that fail
    // parse at resolve time: they must be reported as `skipped` (visible in
    // the report, file untouched) and must NOT make the run fail. No mock
    // server needed — templated and non-hub refs never reach the client.
    let client = RegistryClient::new(Default::default(), std::collections::HashMap::new());
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e4-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ci-templated.yml");
    let original = "job:\n  image: ${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}\nghcr-job:\n  image: ghcr.io/org/img:v1\n";
    std::fs::write(&p, original).unwrap();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: false,
        update_major: false,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    assert_eq!(report.summary.skipped, 2);
    assert_eq!(report.summary.unresolved, 0);
    assert_eq!(report.summary.pinned, 0);
    assert_eq!(exit_code(&report), 0);
    let reasons: Vec<_> = report
        .results
        .iter()
        .map(|r| r.reason.clone().unwrap_or_default())
        .collect();
    assert!(reasons.contains(&"unparseable reference".to_string()));
    assert!(reasons.contains(&"registry not enabled: ghcr.io".to_string()));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn missing_file_is_error() {
    let s = mock_server().await;
    let client = RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([("docker.io".to_string(), s.uri())]),
    );
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: false,
        update_major: false,
    };
    let r = pin_files(&[PathBuf::from("/nonexistent/ci.yml")], &opts, &client).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn update_bumps_scalar_and_pins_digest() {
    let s = MockServer::start().await;
    tag_list_mock("library/alpine", &["3.16", "3.20", "3.24"])
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.24"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D2))
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/nginx/manifests/stable"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D3))
        .mount(&s)
        .await;
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update1-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_update_fixture(&dir);
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    let out = std::fs::read_to_string(&p).unwrap();
    let expected = format!(
        "build:\n  image: alpine:3.24@{D2}\nsidecars:\n  services:\n    - alpine:3.24@{D2}\nplain:\n  image: nginx:stable@{D3}\n"
    );
    assert_eq!(out, expected);
    assert_eq!(report.summary.updated, 2);
    assert_eq!(report.summary.pinned, 1);
    assert_eq!(report.summary.unresolved, 0);
    assert_eq!(exit_code(&report), 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn update_no_bump_when_already_highest() {
    let s = MockServer::start().await;
    tag_list_mock("library/alpine", &["3.20"]).mount(&s).await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&s)
        .await;
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ci.yml");
    std::fs::write(&p, "job:\n  image: alpine:3.20\n").unwrap();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    assert_eq!(report.summary.pinned, 1);
    assert_eq!(report.summary.updated, 0);
    let row = &report.results[0];
    assert_eq!(row.status, "pinned");
    assert_eq!(
        row.new.as_deref(),
        Some(format!("alpine:3.20@{D1}").as_str())
    );
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("job:\n  image: alpine:3.20@{D1}\n")
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn update_same_major_restriction() {
    let s = MockServer::start().await;
    tag_list_mock("library/docker", &["24", "24.0.1", "29"])
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/docker/manifests/24"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D4))
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/docker/manifests/29"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D5))
        .mount(&s)
        .await;
    // One client for both runs: the second run also exercises the
    // per-repo tag-list cache.
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ci.yml");

    // Without update_major: 24 is the highest compatible stable (24.0.1 has
    // a different release length, 29 a different major) → pinned, no bump.
    std::fs::write(&p, "job:\n  image: docker:24\n").unwrap();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    assert_eq!(report.summary.pinned, 1);
    assert_eq!(report.summary.updated, 0);
    assert_eq!(report.results[0].status, "pinned");
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("job:\n  image: docker:24@{D4}\n")
    );

    // With update_major: bumps to 29 and pins 29's digest.
    std::fs::write(&p, "job:\n  image: docker:24\n").unwrap();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: true,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    assert_eq!(report.summary.updated, 1);
    assert_eq!(report.summary.pinned, 0);
    let row = &report.results[0];
    assert_eq!(row.status, "updated");
    assert_eq!(row.new.as_deref(), Some(format!("docker:29@{D5}").as_str()));
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("job:\n  image: docker:29@{D5}\n")
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn update_pinned_never_bumps() {
    let s = MockServer::start().await;
    tag_list_mock("library/alpine", &["3.20", "3.24"])
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&s)
        .await;
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update4-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ci.yml");
    let original = format!("job:\n  image: alpine:3.20@{D1}\n");
    std::fs::write(&p, &original).unwrap();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    assert_eq!(report.summary.ok, 1);
    assert_eq!(report.summary.updated, 0);
    assert_eq!(report.results[0].status, "ok");
    // Pinned refs re-verify only; the tag list is never fetched and the
    // file stays byte-identical.
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn update_tag_list_failure_is_unresolved() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/library/alpine/tags/list"))
        .and(query_param("n", "1000"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&s)
        .await;
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update5-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ci.yml");
    let original = "job:\n  image: alpine:3.20\n";
    std::fs::write(&p, original).unwrap();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    assert_eq!(report.summary.unresolved, 1);
    assert_eq!(exit_code(&report), 1);
    let row = &report.results[0];
    assert_eq!(row.status, "unresolved");
    assert!(
        row.reason.as_deref().unwrap_or("").starts_with("tag list:"),
        "reason: {:?}",
        row.reason
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn update_off_keeps_m1_behavior() {
    let s = MockServer::start().await;
    // No tag-list mock: with update off the tags endpoint must never be
    // touched (wiremock would 404 an unmocked request).
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/nginx/manifests/stable"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D3))
        .mount(&s)
        .await;
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update6-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_update_fixture(&dir);
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: false,
        update_major: false,
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client)
        .await
        .unwrap();
    assert_eq!(report.summary.pinned, 3);
    assert_eq!(report.summary.updated, 0);
    assert_eq!(exit_code(&report), 0);
    let out = std::fs::read_to_string(&p).unwrap();
    let expected = format!(
        "build:\n  image: alpine:3.20@{D1}\nsidecars:\n  services:\n    - alpine:3.20@{D1}\nplain:\n  image: nginx:stable@{D3}\n"
    );
    assert_eq!(out, expected);
    std::fs::remove_dir_all(&dir).ok();
}
