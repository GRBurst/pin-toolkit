//! Opt-in live tests (real Docker Hub). Gate: `PIN_CI_LIVE=1 cargo test --test live`.
//! Without the env var they print a notice and pass without doing anything.
use std::path::Path;
use std::process::Command;

fn client_from_env() -> pin_ci::RegistryClient {
    pin_ci::client_from_env()
}

/// GitLab client for the M3 signature: the live fixtures contain no
/// component/include targets, so it is never contacted (dead local base).
fn gitlab_dummy() -> pin_ci::gitlab::GitLabClient {
    pin_ci::gitlab::GitLabClient::new("http://127.0.0.1:9".into(), None, false)
}

/// `crane digest <ref>` — same unreadable-`~/.docker/config.json`
/// workaround as the oracle (point crane at an empty config dir when the
/// real one is missing or unreadable).
fn crane_digest(ref_name: &str) -> Result<String, String> {
    let mut cmd = Command::new("crane");
    cmd.args(["digest", ref_name]);
    if let Some(home) = std::env::var_os("HOME") {
        let real = Path::new(&home).join(".docker/config.json");
        if std::fs::File::open(&real).is_err() {
            let empty =
                std::env::temp_dir().join(format!("pin-ci-empty-docker-{}", std::process::id()));
            std::fs::create_dir_all(&empty).map_err(|e| e.to_string())?;
            cmd.env("DOCKER_CONFIG", &empty);
        }
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "crane digest {ref_name} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[tokio::test]
async fn live_docker_hub_alpine() {
    if std::env::var("PIN_CI_LIVE").is_err() {
        eprintln!("SKIP live (set PIN_CI_LIVE=1)");
        return;
    }
    use pin_ci::image_ref::ImageRef;
    let c = client_from_env();
    let d1 = c
        .digest(&ImageRef::parse("alpine:3.20").unwrap())
        .await
        .unwrap();
    let d2 = c
        .digest(&ImageRef::parse("alpine:3.20").unwrap())
        .await
        .unwrap();
    assert_eq!(d1, d2);
    assert!(d1.starts_with("sha256:") && d1.len() == 7 + 64, "{d1}");
}

#[tokio::test]
async fn live_docker_hub_ubuntu() {
    if std::env::var("PIN_CI_LIVE").is_err() {
        eprintln!("SKIP live (set PIN_CI_LIVE=1)");
        return;
    }
    use pin_ci::image_ref::ImageRef;
    let d = client_from_env()
        .digest(&ImageRef::parse("ubuntu:24.04").unwrap())
        .await
        .unwrap();
    assert!(d.starts_with("sha256:") && d.len() == 71, "{d}");
}

/// Live `--update --update-major` against real Docker Hub. The tool's
/// choice per reference must equal an independent recomputation (a FRESH
/// client: `list_tags` + `versioning::pick_latest_stable`), and each
/// written digest must equal `crane digest` of the chosen tag. A second
/// run changes nothing (idempotent).
#[tokio::test]
async fn live_update_docker_hub() {
    if std::env::var("PIN_CI_LIVE").is_err() {
        eprintln!("SKIP live (set PIN_CI_LIVE=1)");
        return;
    }
    use pin_ci::{ImageRef, PinOptions, pin_files, versioning};

    // Scratch dir: std::env::temp_dir() when readable+writable; sandboxes
    // where /tmp is write-only get the repo's gitignored .tmp/ instead.
    let tmp = std::env::temp_dir();
    let probe = tmp.join(format!("pin-ci-live-probe-{}", std::process::id()));
    let tmp_ok = std::fs::write(&probe, "x").is_ok() && std::fs::read_to_string(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    let dir = if tmp_ok {
        tmp.join(format!("pin-ci-live-update-{}", std::process::id()))
    } else {
        let alt = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!(".tmp/pin-ci-live-update-{}", std::process::id()));
        if std::fs::create_dir_all(&alt).is_err() {
            eprintln!(
                "SKIP live update (no readable scratch dir; set TMPDIR to a writable, readable dir)"
            );
            return;
        }
        alt
    };
    let yml = dir.join("ci/.gitlab-ci.yml");
    std::fs::create_dir_all(dir.join("ci")).unwrap();
    std::fs::write(&yml, include_str!("fixtures/ci-update-clean.yml")).unwrap();

    let client = client_from_env();
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: true,
        update_major: true,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(std::slice::from_ref(&yml), &opts, &client, &gitlab_dummy())
        .await
        .expect("live update run");

    // Independent recomputation with a FRESH client (own cache, own HTTP).
    let checker = client_from_env();
    let mut mismatches = Vec::new();
    for r in &report.results {
        let parsed = ImageRef::parse(&r.reference).expect("reference parses");
        let current = parsed.effective_tag();
        let repo = parsed.repo.clone();
        let tags = match checker.list_tags(&parsed).await {
            Ok(t) => t,
            Err(e) => {
                mismatches.push(format!("{}: list_tags failed: {e}", r.reference));
                continue;
            }
        };
        let expected_tag = versioning::pick_latest_stable(current, &tags, true)
            .unwrap_or_else(|| current.to_string());
        let want_status = if expected_tag == current {
            "pinned"
        } else {
            "updated"
        };
        let digest = match crane_digest(&format!("{repo}:{expected_tag}")) {
            Ok(d) => d,
            Err(e) => {
                mismatches.push(format!("{repo}:{expected_tag}: crane: {e}"));
                continue;
            }
        };
        let want_token = format!("{repo}:{expected_tag}@{digest}");
        if r.status != want_status {
            mismatches.push(format!(
                "{}: status {} != expected {want_status} (tool chose {})",
                r.reference,
                r.status,
                r.new.as_deref().unwrap_or("<none>")
            ));
        }
        if r.new.as_deref() != Some(want_token.as_str()) {
            mismatches.push(format!(
                "{}: expected token {want_token}, got {:?}",
                r.reference, r.new
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "live update mismatches:\n{}",
        mismatches.join("\n")
    );

    // The written file holds every expected token.
    let written = std::fs::read_to_string(&yml).unwrap();
    for r in &report.results {
        assert!(
            r.new.as_deref().is_some_and(|t| written.contains(t)),
            "file missing token {:?}",
            r.new
        );
    }

    // Second run: all ok, zero changes.
    let report2 = pin_files(std::slice::from_ref(&yml), &opts, &client, &gitlab_dummy())
        .await
        .expect("second run");
    assert_eq!(
        report2.summary.ok,
        report2.results.len(),
        "second run must be all ok:\n{}",
        report2.render_human()
    );
    assert_eq!(
        report2.summary.updated + report2.summary.pinned,
        0,
        "second run must not re-pin or re-update:\n{}",
        report2.render_human()
    );
    assert_eq!(
        std::fs::read_to_string(&yml).unwrap(),
        written,
        "second run must not change the file"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Live ECR public — anonymous, no credentials at all. ECR public issues
/// a Bearer challenge with a trailing-slash realm and a non-repository
/// scope (`scope=aws`); the token request is anonymous and the manifest
/// fetch succeeds without any login (probe-verified 2026-11-06,
/// re-verified 2026-11-09).
///
/// `:latest` digests move over time, so this is a differential check
/// against an independent implementation (`crane digest`), not a frozen
/// value. Cross-check at writing time (2026-11-09 17:00 UTC):
///   DOCKER_CONFIG=<empty dir> crane digest public.ecr.aws/nginx/nginx:latest
///   -> sha256:875df6964525450b1b459b44df8225d2a514a00b72a8e059967e3c38724a486c
#[tokio::test]
async fn live_ecr_public_anonymous() {
    if std::env::var("PIN_CI_LIVE").is_err() {
        eprintln!("SKIP live (set PIN_CI_LIVE=1)");
        return;
    }
    use pin_ci::image_ref::ImageRef;
    const REF: &str = "public.ecr.aws/nginx/nginx:latest";
    let want = match crane_digest(REF) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "SKIP live ecr (crane cross-check unavailable: {e}; hint: put `crane` on PATH, e.g. via the repo devenv)"
            );
            return;
        }
    };
    let got = client_from_env()
        .digest(&ImageRef::parse(REF).unwrap())
        .await
        .expect("anonymous ECR public resolution must succeed");
    assert!(got.starts_with("sha256:") && got.len() == 7 + 64, "{got}");
    assert_eq!(
        got, want,
        "tool digest != crane digest for {REF} (a :latest move between the two calls? re-run)"
    );
}

/// Live GHCR — a real public package, gated on `GITHUB_TOKEN` (R2: the
/// token is sent as `Authorization: Bearer` on the token request).
///
/// GHCR serves the standard Bearer challenge; public packages are also
/// pullable anonymously (probe 2026-11-09 16:01 UTC):
///   curl -sI https://ghcr.io/v2/actions/actions-runner/manifests/latest
///   -> 401, www-authenticate: Bearer realm="https://ghcr.io/token",
///      service="ghcr.io",scope="repository:actions/actions-runner:pull"
///   curl -s 'https://ghcr.io/token?service=ghcr.io&scope=repository:actions/actions-runner:pull'
///   -> 200 {"token":"djE6…"}
///   manifest HEAD with that token -> 200,
///      docker-content-digest:
///      sha256:759a1d94a15b05941aba5196974ff43fe15b4cbba29b694b35cbc32a1166b232
/// This test exercises the token-gated path the plan specifies. The
/// digest is auth-independent (same manifest either way), so the
/// cross-check is an ANONYMOUS `crane digest` — no token plumbing needed
/// for crane. `:latest` moves over time: differential check, not a
/// frozen value.
#[tokio::test]
async fn live_ghcr() {
    if std::env::var("PIN_CI_LIVE").is_err() {
        eprintln!("SKIP live (set PIN_CI_LIVE=1)");
        return;
    }
    if std::env::var("GITHUB_TOKEN")
        .map(|t| t.trim().is_empty())
        .unwrap_or(true)
    {
        eprintln!("SKIP live ghcr (set GITHUB_TOKEN to a PAT with read:packages)");
        return;
    }
    use pin_ci::image_ref::ImageRef;
    const REF: &str = "ghcr.io/actions/actions-runner:latest";
    let want = match crane_digest(REF) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "SKIP live ghcr (crane cross-check unavailable: {e}; hint: put `crane` on PATH, e.g. via the repo devenv)"
            );
            return;
        }
    };
    let got = client_from_env()
        .digest(&ImageRef::parse(REF).unwrap())
        .await
        .expect("GHCR resolution with GITHUB_TOKEN must succeed");
    assert!(got.starts_with("sha256:") && got.len() == 7 + 64, "{got}");
    assert_eq!(
        got, want,
        "tool digest != crane digest for {REF} (a :latest move between the two calls? re-run)"
    );
}
