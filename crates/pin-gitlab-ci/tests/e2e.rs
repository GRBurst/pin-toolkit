use pin_core::oci::RegistryClient;
use pin_gitlab_ci::{PinOptions, exit_code, outcome, pin_files};
use std::path::PathBuf;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const D2: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const D3: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
const D4: &str = "sha256:4444444444444444444444444444444444444444444444444444444444444444";
const D5: &str = "sha256:5555555555555555555555555555555555555555555555555555555555555555";
const D_GHCR: &str = "sha256:6666666666666666666666666666666666666666666666666666666666666666";
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
    // T2 flip: the fixture's ghcr.io line now resolves against the same
    // mock (anonymous 200 digest) instead of being skipped.
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/v1"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D_GHCR))
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
        std::collections::HashMap::from([
            ("docker.io".to_string(), s.uri()),
            // T2: the fixtures' ghcr.io refs resolve, so map them to the mock.
            ("ghcr.io".to_string(), s.uri()),
        ]),
    )
}

/// GitLab client for the M3 signature: the M1/M2 fixtures contain no
/// component/include targets, so it is never contacted (dead local base).
fn gitlab_dummy() -> pin_core::gitlab::GitLabClient {
    pin_core::gitlab::GitLabClient::new("http://127.0.0.1:9".into(), None, false)
}

/// Mock server for the `ci-update.yml` fixture: alpine tag list with 3.24
/// above 3.20, plus digests for alpine 3.20/3.24, nginx:stable, and the
/// fixture's ghcr ref (digest + current-tag-only tag list; T2: every host
/// resolves).
async fn update_mock_server() -> MockServer {
    let s = MockServer::start().await;
    tag_list_mock("library/alpine", &["3.16", "3.20", "3.24"])
        .mount(&s)
        .await;
    // The fixture's ghcr tag `v1` is a parseable version, so --update fetches
    // its tag list too; only the current tag -> no bump, plain pin.
    tag_list_mock("org/img", &["v1"]).mount(&s).await;
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
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/v1"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D_GHCR))
        .mount(&s)
        .await;
    s
}

fn tag_list_mock(repo: &str, tags: &[&str]) -> Mock {
    Mock::given(method("GET"))
        .and(path(format!("/v2/{repo}/tags/list")))
        .and(query_param("n", "1000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"tags": tags})))
}

#[tokio::test]
async fn pins_and_reports_all_states() {
    // T2 flip: the fixture's ghcr.io line is resolved against the mock
    // (base_urls maps ghcr.io -> wiremock) and pinned, no longer skipped.
    let s = mock_server().await;
    let client = RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([
            ("docker.io".to_string(), s.uri()),
            ("ghcr.io".to_string(), s.uri()),
        ]),
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    let out = std::fs::read_to_string(&p).unwrap();
    let expected = format!(
        "image: alpine:3.20@{D1}\nghcr-job:\n  image: ghcr.io/org/img:v1@{D_GHCR}\nmissing-job:\n  image: mystery:1.0\ndup:\n  image: alpine:3.20@{D1}\n"
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
    assert_eq!(by_status.get("pinned"), Some(&3)); // two alpine lines + ghcr
    assert_eq!(by_status.get("skipped"), None); // M1's ghcr skip is gone
    assert_eq!(by_status.get("unresolved"), Some(&1)); // mystery 404
    assert_eq!(exit_code(&report), 1);
    // The flipped expectation: the ghcr.io line is pinned with the mock digest.
    let ghcr = report
        .results
        .iter()
        .find(|r| r.reference == "ghcr.io/org/img:v1")
        .expect("ghcr ref present");
    assert_eq!(ghcr.status, "pinned");
    assert_eq!(
        ghcr.new.as_deref(),
        Some(format!("ghcr.io/org/img:v1@{D_GHCR}").as_str())
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn second_run_is_noop() {
    let s = mock_server().await;
    let client = RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([
            ("docker.io".to_string(), s.uri()),
            ("ghcr.io".to_string(), s.uri()),
        ]),
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let _ = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    let after_first = std::fs::read_to_string(&p).unwrap();
    let report2 = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
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
        std::collections::HashMap::from([
            ("docker.io".to_string(), s.uri()),
            ("ghcr.io".to_string(), s.uri()),
        ]),
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), FIXTURE);
    // T2: the ghcr line resolves too, so three lines would be pinned.
    assert_eq!(report.summary.pinned, 3);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn templated_image_is_skipped_not_unresolved() {
    // Shell/YAML-templated image lines are syntactic candidates that fail
    // parse at resolve time: they must be reported as `skipped` (visible in
    // the report, file untouched) and must NOT make the run fail. No mock
    // server needed — templated refs never reach the client. (T2: the ghcr
    // line that used to be skipped as "registry not enabled" now resolves,
    // so it no longer belongs in this test.)
    let client = RegistryClient::new(Default::default(), std::collections::HashMap::new());
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e4-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ci-templated.yml");
    let original = "job:\n  image: ${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}\n";
    std::fs::write(&p, original).unwrap();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: false,
        update_major: false,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report.summary.skipped, 1);
    assert_eq!(report.summary.unresolved, 0);
    assert_eq!(report.summary.pinned, 0);
    assert_eq!(exit_code(&report), 0);
    let reasons: Vec<_> = report
        .results
        .iter()
        .map(|r| r.reason.clone().unwrap_or_default())
        .collect();
    assert!(reasons.contains(&"unparseable reference".to_string()));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
    std::fs::remove_dir_all(&dir).ok();
}

/// Review L2: a file error still stops the run, but the rows of the files
/// before it are kept and printed; the exit code stays 1.
#[tokio::test]
async fn file_error_keeps_the_rows_of_earlier_files() {
    let client = RegistryClient::new(Default::default(), std::collections::HashMap::new());
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-partial-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.yml");
    let bad = dir.join("bad.yml");
    let later = dir.join("later.yml");
    let text = "job:\n  image: ${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}\n";
    std::fs::write(&good, text).unwrap();
    std::fs::write(&bad, "image: alpine:3.20\nb: c: d\n").unwrap();
    std::fs::write(&later, text).unwrap();
    let opts = PinOptions::default();
    let paths = [good.clone(), bad.clone(), later];
    let err = pin_files(&paths, &opts, &client, &gitlab_dummy())
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "{}:2: invalid YAML: mapping values are not allowed in this context",
            bad.display()
        )
    );
    assert_eq!(err.partial.results.len(), 1);
    assert_eq!(err.partial.results[0].file, good.display().to_string());
    assert_eq!(err.partial.summary.skipped, 1);

    let out = outcome(Err(err), false);
    assert_eq!(out.code, 1);
    let stdout = out.stdout.expect("rows of the good file");
    assert!(
        stdout.starts_with(&format!(
            "{}:2  skipped (unparseable reference)",
            good.display()
        )),
        "{stdout}"
    );
    assert!(stdout.ends_with("pinned: 0, ok: 0, updated: 0, skipped: 1, unresolved: 0\n"));
    assert!(out.stderr.unwrap().starts_with("pin-ci: "));
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let r = pin_files(
        &[PathBuf::from("/nonexistent/ci.yml")],
        &opts,
        &client,
        &gitlab_dummy(),
    )
    .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn update_bumps_scalar_and_pins_digest() {
    let s = MockServer::start().await;
    tag_list_mock("library/alpine", &["3.16", "3.20", "3.24"])
        .mount(&s)
        .await;
    // The fixture's ghcr tag `v1` is a parseable version, so --update fetches
    // its tag list too; only the current tag -> no bump, plain pin.
    tag_list_mock("org/img", &["v1"]).mount(&s).await;
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
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/v1"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D_GHCR))
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    let out = std::fs::read_to_string(&p).unwrap();
    // T2: the fixture's ghcr ref resolves too; its tag list holds only the
    // current tag, so it pins without bumping.
    let expected = format!(
        "build:\n  image: alpine:3.24@{D2}\nsidecars:\n  services:\n    - alpine:3.24@{D2}\nplain:\n  image: nginx:stable@{D3}\nexternal:\n  image: ghcr.io/org/img:v1@{D_GHCR}\n"
    );
    assert_eq!(out, expected);
    assert_eq!(report.summary.updated, 2);
    assert_eq!(report.summary.pinned, 2);
    assert_eq!(report.summary.skipped, 0);
    assert_eq!(report.summary.unresolved, 0);
    assert_eq!(exit_code(&report), 0);
    std::fs::remove_dir_all(&dir).ok();
}

/// T2: `--update` works on a non-hub mock host — the tag list and digest
/// lookups go through the same per-host base/auth path as Docker Hub.
#[tokio::test]
async fn update_works_on_nonhub() {
    let s = MockServer::start().await;
    tag_list_mock("org/img", &["1.0", "1.2", "2.0"])
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/1.0"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&s)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/1.2"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D2))
        .mount(&s)
        .await;
    let client = RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([("ghcr.io".to_string(), s.uri())]),
    );
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update12-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ci.yml");
    std::fs::write(&p, "job:\n  image: ghcr.io/org/img:1.0\n").unwrap();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report.summary.updated, 1);
    assert_eq!(report.summary.unresolved, 0);
    let row = &report.results[0];
    assert_eq!(row.status, "updated");
    // Same-major bump 1.0 -> 1.2 (2.0 is a higher major) + digest pin.
    assert_eq!(
        row.new.as_deref(),
        Some(format!("ghcr.io/org/img:1.2@{D2}").as_str())
    );
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("job:\n  image: ghcr.io/org/img:1.2@{D2}\n")
    );
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
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
async fn update_json_report() {
    let s = update_mock_server().await;
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update7-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_update_fixture(&dir);
    let opts = PinOptions {
        dry_run: false,
        json: true,
        concurrency: 4,
        update: true,
        update_major: false,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(v["summary"]["updated"], 2);
    let results = v["results"].as_array().unwrap();
    let statuses: Vec<&str> = results
        .iter()
        .map(|r| r["status"].as_str().unwrap())
        .collect();
    assert!(statuses.contains(&"updated"));
    assert!(statuses.contains(&"pinned"));
    let want = format!("alpine:3.24@{D2}");
    let alpine_news: Vec<String> = results
        .iter()
        .filter(|r| r["status"] == "updated")
        .map(|r| r["new"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(alpine_news, vec![want.clone(), want]);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn update_human_report_line() {
    let s = update_mock_server().await;
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update8-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_update_fixture(&dir);
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    let out = report.render_human();
    let want_line = format!("  updated  alpine:3.20 -> alpine:3.24@{D2}  # alpine:3.20");
    assert!(
        out.lines().any(|l| l.contains(&want_line)),
        "human output:\n{out}"
    );
    // T2: the fixture's ghcr ref pins instead of skipping.
    assert!(
        out.lines()
            .any(|l| l.ends_with("pinned: 2, ok: 0, updated: 2, skipped: 0, unresolved: 0")),
        "human output:\n{out}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn update_dry_run_does_not_write() {
    let s = update_mock_server().await;
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update9-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_update_fixture(&dir);
    let opts = PinOptions {
        dry_run: true,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    // The report shows what would change... (T2: ghcr pins, none skipped)
    assert_eq!(report.summary.updated, 2);
    assert_eq!(report.summary.pinned, 2);
    assert_eq!(report.summary.skipped, 0);
    // ...but the file bytes are untouched.
    assert_eq!(std::fs::read_to_string(&p).unwrap(), UPDATE_FIXTURE);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn update_idempotent_second_run() {
    let s = update_mock_server().await;
    let client = mock_client(&s);
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update10-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_update_fixture(&dir);
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report1 = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report1.summary.updated, 2);
    let after_first = std::fs::read_to_string(&p).unwrap();
    let report2 = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    // Pinned refs re-verify only: no second bump, no second write.
    // T2: the ghcr line re-verifies as ok too.
    assert_eq!(report2.summary.updated, 0);
    assert_eq!(report2.summary.ok, 4);
    assert_eq!(report2.summary.pinned, 0);
    assert!(
        report2
            .results
            .iter()
            .all(|r| r.status == "ok" || r.status == "skipped"),
        "statuses: {:?}",
        report2
            .results
            .iter()
            .map(|r| r.status.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), after_first);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn update_exit_code() {
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-update11-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: false,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };

    // Mix of updated + pinned: nothing unresolved → exit 0.
    let s = update_mock_server().await;
    let client = mock_client(&s);
    let p = write_update_fixture(&dir);
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report.summary.updated, 2);
    assert_eq!(report.summary.skipped, 0); // T2: ghcr pins, nothing skips
    assert_eq!(report.summary.unresolved, 0);
    assert_eq!(exit_code(&report), 0);

    // Same file with a failing tags endpoint: both alpine refs become
    // unresolved → exit 1, file untouched.
    let s2 = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/library/alpine/tags/list"))
        .and(query_param("n", "1000"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&s2)
        .await;
    // The ghcr ref still resolves on this server (no bump, plain pin).
    tag_list_mock("org/img", &["v1"]).mount(&s2).await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&s2)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/nginx/manifests/stable"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D3))
        .mount(&s2)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/v1"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D_GHCR))
        .mount(&s2)
        .await;
    let client2 = mock_client(&s2);
    std::fs::write(&p, UPDATE_FIXTURE).unwrap();
    let report2 = pin_files(std::slice::from_ref(&p), &opts, &client2, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report2.summary.unresolved, 2);
    assert_eq!(exit_code(&report2), 1);
    // M1 partial-write semantics: the failing alpine refs stay as written;
    // the resolvable nginx and ghcr refs are still pinned and written.
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!(
            "build:\n  image: alpine:3.20\nsidecars:\n  services:\n    - alpine:3.20\nplain:\n  image: nginx:stable@{D3}\nexternal:\n  image: ghcr.io/org/img:v1@{D_GHCR}\n"
        )
    );
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
    Mock::given(method("HEAD"))
        .and(path("/v2/org/img/manifests/v1"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D_GHCR))
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
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    // T2: the ghcr line pins too (update off: no tag list, straight pin).
    assert_eq!(report.summary.pinned, 4);
    assert_eq!(report.summary.updated, 0);
    assert_eq!(report.summary.skipped, 0);
    assert_eq!(exit_code(&report), 0);
    let out = std::fs::read_to_string(&p).unwrap();
    let expected = format!(
        "build:\n  image: alpine:3.20@{D1}\nsidecars:\n  services:\n    - alpine:3.20@{D1}\nplain:\n  image: nginx:stable@{D3}\nexternal:\n  image: ghcr.io/org/img:v1@{D_GHCR}\n"
    );
    assert_eq!(out, expected);
    std::fs::remove_dir_all(&dir).ok();
}

// ---- M4b-1 Task 2: release-age guard e2e (wiremock registry + hub) ----

/// Guard-fixture Docker Hub repo (the guard is docker.io-only, R3).
const GUARD_REPO: &str = "acme/app";

/// Far-past / far-fixture push timestamps so wall-clock can't flip
/// outcomes (plan constraint 8).
const TS_OLD: &str = "2020-01-01T00:00:00Z";
const TS_YOUNG: &str = "2999-01-01T00:00:00Z";

/// Hub API mock: tag → `tag_last_pushed` (None = tag absent from the
/// Hub listing, e.g. unparsed timestamp).
fn hub_ts_mock(repo: &str, entries: &[(&str, Option<&str>)]) -> Mock {
    let results: Vec<serde_json::Value> = entries
        .iter()
        .map(|(name, ts)| match ts {
            Some(ts) => serde_json::json!({ "name": name, "tag_last_pushed": ts }),
            None => serde_json::json!({ "name": name }),
        })
        .collect();
    Mock::given(method("GET"))
        .and(path(format!("/v2/repositories/{repo}/tags")))
        .and(query_param("page_size", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "count": results.len(),
            "next": null,
            "results": results
        })))
}

/// Digest (HEAD) mock for one repo tag on the registry base.
fn digest_mock(repo: &str, tag: &str, digest: &str) -> Mock {
    Mock::given(method("HEAD"))
        .and(path(format!("/v2/{repo}/manifests/{tag}")))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", digest))
}

/// Per-test scratch dir under the repo's gitignored `.tmp/`.
fn guard_tmp(name: &str) -> PathBuf {
    let dir = std::path::Path::new(".tmp")
        .join(format!("pin-ci-e2e-guard-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Registry mock with the guard's docker.io base plus the Hub mock on a
/// separate server (the plan's registry + hub wiremock pattern).
async fn guard_client(
    tags: &[&str],
    hub_entries: &[(&str, Option<&str>)],
    digests: &[(&str, &str)],
) -> (RegistryClient, MockServer, MockServer) {
    let s = MockServer::start().await;
    tag_list_mock(GUARD_REPO, tags).mount(&s).await;
    for (tag, digest) in digests {
        digest_mock(GUARD_REPO, tag, digest).mount(&s).await;
    }
    let hub = MockServer::start().await;
    hub_ts_mock(GUARD_REPO, hub_entries).mount(&hub).await;
    let client = RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([("docker.io".to_string(), s.uri())]),
    )
    .with_hub_base(&hub.uri());
    (client, s, hub)
}

async fn guard_file(name: &str, ref_str: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = guard_tmp(name);
    let p = dir.join("ci.yml");
    std::fs::write(&p, format!("job:\n  image: {ref_str}\n")).unwrap();
    (p, dir)
}

/// The youngest tag (1.4, far-future push ts) is too new for the 14d
/// guard; the oldest old-enough candidate (1.3) wins, digest-pinned.
#[tokio::test]
async fn guard_young_top_dropped() {
    let (client, _s, _hub) = guard_client(
        &["1.1", "1.2", "1.3", "1.4"],
        &[
            ("1.1", Some(TS_OLD)),
            ("1.2", Some(TS_OLD)),
            ("1.3", Some(TS_OLD)),
            ("1.4", Some(TS_YOUNG)), // too young → dropped
        ],
        &[("1.2", D2), ("1.3", D3)],
    )
    .await;
    let (p, dir) = guard_file("young-top", "acme/app:1.2").await;
    let opts = PinOptions {
        update: true,
        min_age_days: 14,
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report.summary.updated, 1);
    assert_eq!(report.summary.skipped, 0);
    assert_eq!(report.summary.unresolved, 0);
    assert_eq!(
        report.results[0].new.as_deref(),
        Some(format!("acme/app:1.3@{D3}").as_str())
    );
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("job:\n  image: acme/app:1.3@{D3}\n")
    );
    assert_eq!(exit_code(&report), 0);
    std::fs::remove_dir_all(&dir).ok();
}

/// Every bump candidate is younger than 14d → R5 skip: the ref is left
/// byte-identical, reported `skipped (release age: …)`, exit 0.
#[tokio::test]
async fn guard_no_eligible_skip() {
    let (client, _s, _hub) = guard_client(
        &["1.1", "1.2", "1.3"],
        &[
            ("1.1", Some(TS_OLD)),
            ("1.2", Some(TS_YOUNG)), // too young
            ("1.3", Some(TS_YOUNG)), // too young
        ],
        &[("1.1", D1)],
    )
    .await;
    let (p, dir) = guard_file("no-eligible", "acme/app:1.1").await;
    let original = "job:\n  image: acme/app:1.1\n";
    let opts = PinOptions {
        update: true,
        min_age_days: 14,
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report.summary.skipped, 1);
    assert_eq!(report.summary.updated, 0);
    assert_eq!(report.summary.unresolved, 0);
    assert_eq!(report.results[0].status, "skipped");
    assert_eq!(
        report.results[0].reason.as_deref(),
        Some("release age: no candidate ≥ 14d")
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
    assert_eq!(exit_code(&report), 0);
    std::fs::remove_dir_all(&dir).ok();
}

/// A tag with no Hub timestamp entry is NOT old enough (R6,
/// timestamp-required parity): 1.3 is excluded, 1.2 wins.
#[tokio::test]
async fn guard_missing_timestamp_excluded() {
    let (client, _s, _hub) = guard_client(
        &["1.1", "1.2", "1.3"],
        &[
            ("1.1", Some(TS_OLD)),
            ("1.2", Some(TS_OLD)),
            ("1.3", None), // absent from the Hub listing → excluded
        ],
        &[("1.1", D1), ("1.2", D2)],
    )
    .await;
    let (p, dir) = guard_file("missing-ts", "acme/app:1.1").await;
    let opts = PinOptions {
        update: true,
        min_age_days: 14,
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report.summary.updated, 1);
    assert_eq!(report.summary.skipped, 0);
    assert_eq!(report.results[0].status, "updated");
    assert_eq!(
        report.results[0].new.as_deref(),
        Some(format!("acme/app:1.2@{D2}").as_str())
    );
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("job:\n  image: acme/app:1.2@{D2}\n")
    );
    assert_eq!(exit_code(&report), 0);
    std::fs::remove_dir_all(&dir).ok();
}

/// Same fixture as `guard_young_top_dropped`, but the flag is absent
/// (min_age_days = 0, the default): pure M2 — the youngest tag wins and
/// the Hub API is never called.
#[tokio::test]
async fn guard_off_default_m2_unchanged() {
    let (client, _s, hub) = guard_client(
        &["1.1", "1.2", "1.3", "1.4"],
        &[
            ("1.1", Some(TS_OLD)),
            ("1.2", Some(TS_OLD)),
            ("1.3", Some(TS_OLD)),
            ("1.4", Some(TS_YOUNG)),
        ],
        &[("1.2", D2), ("1.4", D4)],
    )
    .await;
    let (p, dir) = guard_file("off-default", "acme/app:1.2").await;
    let opts = PinOptions {
        update: true,
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report.summary.updated, 1);
    assert_eq!(report.results[0].status, "updated");
    assert_eq!(
        report.results[0].new.as_deref(),
        Some(format!("acme/app:1.4@{D4}").as_str())
    );
    // R4: with the flag off the behavior is M2 — the Hub API is a
    // no-op, zero requests.
    assert!(hub.received_requests().await.unwrap().is_empty());
    assert_eq!(exit_code(&report), 0);
    std::fs::remove_dir_all(&dir).ok();
}

/// R3: a non-docker.io host is a documented no-op — M2 behavior
/// verbatim and the Hub API is never called (zero requests).
#[tokio::test]
async fn guard_nonhub_noop() {
    let s = MockServer::start().await;
    tag_list_mock("org/img", &["1.0", "1.2"]).mount(&s).await;
    digest_mock("org/img", "1.0", D1).mount(&s).await;
    digest_mock("org/img", "1.2", D2).mount(&s).await;
    let hub = MockServer::start().await; // nothing mounted: any call would 404
    let client = RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([("ghcr.io".to_string(), s.uri())]),
    )
    .with_hub_base(&hub.uri());
    let dir = guard_tmp("nonhub");
    let p = dir.join("ci.yml");
    std::fs::write(&p, "job:\n  image: ghcr.io/org/img:1.0\n").unwrap();
    let opts = PinOptions {
        update: true,
        min_age_days: 14,
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&p), &opts, &client, &gitlab_dummy())
        .await
        .unwrap();
    assert_eq!(report.summary.updated, 1);
    assert_eq!(report.summary.skipped, 0);
    assert_eq!(report.summary.unresolved, 0);
    assert_eq!(report.results[0].status, "updated");
    assert_eq!(
        report.results[0].new.as_deref(),
        Some(format!("ghcr.io/org/img:1.2@{D2}").as_str())
    );
    assert!(hub.received_requests().await.unwrap().is_empty());
    assert_eq!(exit_code(&report), 0);
    std::fs::remove_dir_all(&dir).ok();
}
