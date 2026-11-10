//! M3 Task 4 e2e: component + include pinning.
//!
//! wiremock registry (digests) + wiremock GitLab (tags/branches) against a
//! temp CI file in the repo's gitignored `.tmp/` scratch.
use std::path::{Path, PathBuf};

use pin_core::gitlab::GitLabClient;
use pin_core::oci::RegistryClient;
use pin_core::report::Status;
use pin_gitlab_ci::{PinOptions, exit_code, pin_files};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// 40-hex lowercase commit SHAs.
const SHA1: &str = "e3262fdd0914fa823210cdb79a8c421e2cef79d8";
const SHA2: &str = "4f0a3b6c1d8e2f5a7b0c3d6e9f1a2b5c8d0e3f60";

const MIXED: &str = "\
image: alpine:3.20
component: gitlab.com/g/p/name@1.0.0
include:
  project: g/p
  ref: main
";

/// Registry mock: alpine:3.20 → D1.
async fn registry_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/v2/library/alpine/manifests/3.20"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1))
        .mount(&s)
        .await;
    s
}

/// GitLab mock for project `g/p`: tag `1.0.0` (a FULL numeric version
/// resolves as a tag, not catalog-managed) → SHA1; branch `main` →
/// SHA2 (tag `main` stays unmocked so it 404s and the bare-name fallback
/// reaches the branch endpoint).
async fn gitlab_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/tags/1.0.0"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"commit": {"id": SHA1}})),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/projects/g%2Fp/repository/branches/main"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"commit": {"id": SHA2}})),
        )
        .mount(&s)
        .await;
    s
}

fn registry_client(s: &MockServer) -> RegistryClient {
    RegistryClient::new(
        Default::default(),
        std::collections::HashMap::from([("docker.io".to_string(), s.uri())]),
    )
}

fn gitlab_client(s: &MockServer) -> GitLabClient {
    GitLabClient::new(s.uri(), None, false)
}

fn opts(dry_run: bool) -> PinOptions {
    PinOptions {
        dry_run,
        json: false,
        concurrency: 4,
        update: false,
        update_major: false,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    }
}

/// Per-test temp dir under the repo's gitignored `.tmp/` scratch.
fn tmp_dir(name: &str) -> PathBuf {
    let dir = Path::new(".tmp").join(format!("pin-ci-e2e-c-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_ci(dir: &Path, content: &str) -> PathBuf {
    let p = dir.join("ci.yml");
    std::fs::write(&p, content).unwrap();
    p
}

/// 1. Image + component + include(block) in one file → 3 pinned;
///    byte-exact output incl. the original-ref trailing comments.
#[tokio::test]
async fn mixed_file_all_pinned() {
    let rs = registry_server().await;
    let gs = gitlab_server().await;
    let reg = registry_client(&rs);
    let gitlab = gitlab_client(&gs);
    let dir = tmp_dir("mixed");
    let p = write_ci(&dir, MIXED);
    let report = pin_files(std::slice::from_ref(&p), &opts(false), &reg, &gitlab)
        .await
        .unwrap();
    assert_eq!(report.summary.pinned, 3);
    assert_eq!(exit_code(&report), 0);
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!(
            "image: alpine:3.20@{D1}\ncomponent: gitlab.com/g/p/name@{SHA1}  # 1.0.0\ninclude:\n  project: g/p\n  ref: {SHA2}  # main\n"
        )
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 2. Second run: everything already a SHA/digest → all `ok`, zero writes.
#[tokio::test]
async fn idempotent_second_run() {
    let rs = registry_server().await;
    let gs = gitlab_server().await;
    let reg = registry_client(&rs);
    let gitlab = gitlab_client(&gs);
    let dir = tmp_dir("idempotent");
    let p = write_ci(&dir, MIXED);
    let report1 = pin_files(std::slice::from_ref(&p), &opts(false), &reg, &gitlab)
        .await
        .unwrap();
    assert_eq!(report1.summary.pinned, 3);
    let after_first = std::fs::read_to_string(&p).unwrap();
    let report2 = pin_files(std::slice::from_ref(&p), &opts(false), &reg, &gitlab)
        .await
        .unwrap();
    assert_eq!(
        report2.summary.ok,
        report2.results.len(),
        "second run must be all ok: {}",
        report2.render_human()
    );
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        after_first,
        "second run must not change the file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 3. All skip rules → exit 0, no writes, no HTTP at all.
#[tokio::test]
async fn skips_keep_exit_zero() {
    let reg = RegistryClient::new(Default::default(), std::collections::HashMap::new());
    // Never contacted: nothing in the file reaches a client.
    let gitlab = GitLabClient::new("http://127.0.0.1:9".into(), None, false);
    let dir = tmp_dir("skips");
    let original = "\
component: ghcr.io/g/p/name@1.0.0
component: gitlab.com/g/p/name@1.0.x
include:
  project: g/p
  ref: ${VAR}
component: gitlab.com/g/p/name
";
    let p = write_ci(&dir, original);
    let report = pin_files(std::slice::from_ref(&p), &opts(false), &reg, &gitlab)
        .await
        .unwrap();
    assert_eq!(report.summary.skipped, 4);
    assert_eq!(report.summary.pinned, 0);
    assert_eq!(report.summary.unresolved, 0);
    assert_eq!(exit_code(&report), 0);
    let reasons: Vec<_> = report
        .results
        .iter()
        .map(|r| r.reason.clone().unwrap_or_default())
        .collect();
    assert!(reasons.contains(&"host not enabled: ghcr.io".to_string()));
    assert!(reasons.contains(&"catalog-managed version".to_string()));
    assert!(reasons.contains(&"templated reference".to_string()));
    assert!(reasons.contains(&"unparseable reference".to_string()));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 4. One include 404s on tag AND branch → `unresolved` → exit 1; the
///    resolvable image is still pinned and written (partial write).
#[tokio::test]
async fn unresolved_exit_one() {
    let rs = registry_server().await;
    // No gitlab mocks at all: tag and branch lookups both 404.
    let gs = MockServer::start().await;
    let reg = registry_client(&rs);
    let gitlab = gitlab_client(&gs);
    let dir = tmp_dir("unresolved");
    let p = write_ci(
        &dir,
        "image: alpine:3.20\ninclude:\n  project: g/p\n  ref: missing\n",
    );
    let report = pin_files(std::slice::from_ref(&p), &opts(false), &reg, &gitlab)
        .await
        .unwrap();
    assert_eq!(report.summary.pinned, 1);
    assert_eq!(report.summary.unresolved, 1);
    assert_eq!(exit_code(&report), 1);
    let row = report
        .results
        .iter()
        .find(|r| r.status == Status::Unresolved)
        .unwrap();
    assert_eq!(
        row.reason.as_deref(),
        Some("g/p:missing not found (no tag or branch)")
    );
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("image: alpine:3.20@{D1}\ninclude:\n  project: g/p\n  ref: missing\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 5. A `ref:` line that already carries a trailing comment keeps it
///    untouched — no second comment is added.
#[tokio::test]
async fn existing_comment_unchanged() {
    let gs = gitlab_server().await;
    let reg = RegistryClient::new(Default::default(), std::collections::HashMap::new());
    let gitlab = gitlab_client(&gs);
    let dir = tmp_dir("comment");
    let p = write_ci(&dir, "include:\n  project: g/p\n  ref: main  # keep\n");
    let report = pin_files(std::slice::from_ref(&p), &opts(false), &reg, &gitlab)
        .await
        .unwrap();
    assert_eq!(report.summary.pinned, 1);
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("include:\n  project: g/p\n  ref: {SHA2}  # keep\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 6. Flow include: the comment lands after the closing bracket.
#[tokio::test]
async fn flow_include_comment_after_bracket() {
    let gs = gitlab_server().await;
    let reg = RegistryClient::new(Default::default(), std::collections::HashMap::new());
    let gitlab = gitlab_client(&gs);
    let dir = tmp_dir("flow");
    let p = write_ci(&dir, "include: { project: g/p, ref: main }\n");
    let report = pin_files(std::slice::from_ref(&p), &opts(false), &reg, &gitlab)
        .await
        .unwrap();
    assert_eq!(report.summary.pinned, 1);
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("include: {{ project: g/p, ref: {SHA2} }}  # main\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 7. `--dry-run`: statuses are reported, the file is untouched.
#[tokio::test]
async fn dry_run_no_write() {
    let rs = registry_server().await;
    let gs = gitlab_server().await;
    let reg = registry_client(&rs);
    let gitlab = gitlab_client(&gs);
    let dir = tmp_dir("dryrun");
    let p = write_ci(&dir, MIXED);
    let report = pin_files(std::slice::from_ref(&p), &opts(true), &reg, &gitlab)
        .await
        .unwrap();
    assert_eq!(report.summary.pinned, 3);
    assert_eq!(exit_code(&report), 0);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), MIXED);
    let _ = std::fs::remove_dir_all(&dir);
}
