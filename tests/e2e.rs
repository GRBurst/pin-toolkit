use pin_ci::{PinOptions, RegistryClient, exit_code, pin_files};
use std::path::PathBuf;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FIXTURE: &str = include_str!("fixtures/ci-e2e.yml");

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
    };
    let r = pin_files(&[PathBuf::from("/nonexistent/ci.yml")], &opts, &client).await;
    assert!(r.is_err());
}
