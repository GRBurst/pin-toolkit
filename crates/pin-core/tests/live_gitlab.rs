//! Opt-in live tests (real gitlab.com, anonymous or PAT). Gate:
//! `PIN_CI_LIVE=1 cargo test --test live_gitlab`.
//! Without the env var they print a notice and pass without doing anything.
//!
//! There is no renovate oracle for components/includes (renovate does not
//! handle them — recon §7); the independent oracle here is `git ls-remote`,
//! which speaks plain git protocol instead of the GitLab REST API.
use std::process::Command;

const GITLAB_API: &str = "https://gitlab.com/api/v4";
const GITLAB_URL: &str = "https://gitlab.com";

/// Long-lived public project (the GitLab codebase itself).
const PROJECT: &str = "gitlab-org/gitlab-foss";
/// Old, immutable tag (v13.0.0, 2020), ANNOTATED: `git ls-remote` reports
/// the tag object `805e1869acf70a3b624a09a288e50de8b5b9e106`, while the
/// client (and GitLab CI) resolves it to the underlying commit
/// `6743e883eae128226604b9fad2faead6fcc7ae75`. Verified at implementation
/// time with the peel (see `git_ls_remote_sha`):
/// `git ls-remote https://gitlab.com/gitlab-org/gitlab-foss 'refs/tags/v13.0.0^{*}' 'refs/heads/main'`
///   → `6743e883eae128226604b9fad2faead6fcc7ae75  refs/tags/v13.0.0^{}`
const TAG: &str = "refs/tags/v13.0.0";
/// Oracle request pattern for the tag: `^{*}` peels the annotated tag to its
/// commit, matching what `resolve` returns (the commit SHA, not the tag
/// object SHA).
const TAG_PATTERN: &str = "refs/tags/v13.0.0^{*}";
/// The peeled tag as printed in `git ls-remote` output (`<sha>\t<ref>`).
const TAG_OUT_REF: &str = "refs/tags/v13.0.0^{}";
/// Long-lived default branch. Same `git ls-remote` command verified it
/// resolves (to `6bcec1c443162e6b97e9589002994c76711b8c11` at verification
/// time); `main` moves over time, so the branch test tolerates a push
/// landing during the run (see `live_gitlab_resolves_branch`).
const BRANCH: &str = "refs/heads/main";
/// 40-hex commit SHA (the commit the v13.0.0 tag points to): resolution
/// must return it unchanged without any API call.
const PINNED_SHA: &str = "6743e883eae128226604b9fad2faead6fcc7ae75";

/// Print the remedy hint and return `true` when the test must skip.
fn skip_unless_live() -> bool {
    if std::env::var("PIN_CI_LIVE").is_err() {
        eprintln!("SKIP live gitlab (set PIN_CI_LIVE=1)");
        return true;
    }
    false
}

/// `git` must be on PATH (it is in the devenv); skip with a hint otherwise.
fn skip_unless_git() -> bool {
    match Command::new("git").arg("--version").output() {
        Ok(_) => false,
        Err(e) => {
            eprintln!(
                "SKIP live gitlab (git not on PATH: {e}; add git to PATH — it is in the devenv)"
            );
            true
        }
    }
}

/// The commit SHA that `out_ref` points to per `git ls-remote` of the fixed
/// project — the independent oracle. Runs
/// `git ls-remote <url> 'refs/tags/v13.0.0^{*}' refs/heads/main` and parses
/// the `<sha>\t<ref>` line whose ref name is exactly `out_ref` (peeled tag
/// refs print as `refs/tags/t^{}`).
fn git_ls_remote_sha(out_ref: &str) -> Result<String, String> {
    let out = Command::new("git")
        .args([
            "ls-remote",
            &format!("{GITLAB_URL}/{PROJECT}"),
            TAG_PATTERN,
            BRANCH,
        ])
        .output()
        .map_err(|e| format!("git ls-remote failed to start: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-remote failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some((sha, refname)) = line.split_once('\t') else {
            continue;
        };
        if refname == out_ref {
            return Ok(sha.trim().to_string());
        }
    }
    Err(format!(
        "{out_ref} missing from git ls-remote output (constants are stale?)"
    ))
}

/// The tag is immutable: one comparison is decisive.
#[tokio::test]
async fn live_gitlab_resolves_tag() {
    if skip_unless_live() || skip_unless_git() {
        return;
    }
    let want = git_ls_remote_sha(TAG_OUT_REF).expect("git ls-remote oracle");
    let client = pin_core::gitlab::GitLabClient::from_env(GITLAB_API.to_string());
    let got = client
        .resolve(PROJECT, TAG)
        .await
        .unwrap_or_else(|e| panic!("resolve({PROJECT}, {TAG}) failed: {e}"));
    assert_eq!(got, want, "tag: GitLab API disagrees with git ls-remote");
}

/// `main` is a live branch: a push between the `git ls-remote` read and the
/// API read can legitimately move it. A FRESH client per attempt (the client
/// caches per `(project, ref)`) and a fresh oracle read; two consecutive
/// straddled pushes would make the final attempt fail, which is the
/// intended loud failure (the cross-check would be untrustworthy).
#[tokio::test]
async fn live_gitlab_resolves_branch() {
    if skip_unless_live() || skip_unless_git() {
        return;
    }
    let mut last = (String::new(), String::new());
    for attempt in 1..=2 {
        let want = git_ls_remote_sha(BRANCH).expect("git ls-remote oracle");
        let client = pin_core::gitlab::GitLabClient::from_env(GITLAB_API.to_string());
        let got = client
            .resolve(PROJECT, BRANCH)
            .await
            .unwrap_or_else(|e| panic!("resolve({PROJECT}, {BRANCH}) failed: {e}"));
        if got == want {
            return;
        }
        eprintln!("branch moved during attempt {attempt} (git={want} api={got}); retrying");
        last = (want, got);
    }
    panic!(
        "branch: two consecutive mismatches (git={} api={}): real discrepancy, not a move",
        last.0, last.1
    );
}

/// A pinned 40-hex SHA short-circuits: returned unchanged, zero API calls
/// (the zero-request property itself is covered offline by
/// `sha_short_circuit` in `src/gitlab.rs` with a wiremock server).
#[tokio::test]
async fn live_gitlab_pinned_sha_short_circuit() {
    if skip_unless_live() {
        return;
    }
    let client = pin_core::gitlab::GitLabClient::from_env(GITLAB_API.to_string());
    let got = client
        .resolve(PROJECT, PINNED_SHA)
        .await
        .unwrap_or_else(|e| panic!("resolve({PROJECT}, {PINNED_SHA}) failed: {e}"));
    assert_eq!(got, PINNED_SHA, "a 40-hex SHA must be returned unchanged");
}
