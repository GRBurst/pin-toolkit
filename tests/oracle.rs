//! Differential oracle test against the old pipeline (owner's required gate).
//!
//! Gate: run with `PIN_CI_ORACLE=1 cargo test --test oracle` and with both
//! `renovate` and `crane` on `PATH`. Otherwise the test prints a notice and
//! passes without doing anything.
//!
//! Semantics: the old pipeline does not only pin — it rewrites each image to
//! renovate's `newValue` (a tag bump) and then pins that tag's digest via
//! `crane`. M1 is pin-only (tag bumping is M2's `--update`), so a whole-file
//! digest-set comparison would fail by design whenever renovate reports an
//! update. The oracle therefore compares what both tools must agree on: for
//! every `image:tag@sha256:...` the old pipeline wrote, the new tool's
//! resolver must return the *same digest* for the *same* `image:tag`.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use pin_ci::{ImageRef, PinOptions, client_from_env, pin_files, versioning};

/// Repo root at compile time; the old pipeline runs as-is from here.
const REPO: &str = env!("CARGO_MANIFEST_DIR");

const CI_YAML: &str = "\
image: alpine:3.20
build:
  image: ubuntu:24.04
  services:
    - docker:24
";

/// GitLab client for the M3 signature: the oracle fixtures contain no
/// component/include targets, so it is never contacted (dead local base).
fn gitlab_dummy() -> pin_ci::gitlab::GitLabClient {
    pin_ci::gitlab::GitLabClient::new("http://127.0.0.1:9".into(), None, false)
}

/// True when `name` is on PATH (checked `which`-style via `command -v`).
fn bin_present(name: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {name}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Extract `image_ref -> "sha256:<64hex>"` pairs from `text`: for each
/// `@sha256:` followed by 64 hex digits, the image ref is the longest run of
/// `[\w./-]` immediately before it (manual scan, no regex dependency).
fn pinned_pairs(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(pos) = text[i..].find("@sha256:") {
        let at = i + pos;
        let hex_start = at + 8; // "@sha256:" is 8 chars
        if hex_start + 64 <= bytes.len()
            && text[hex_start..hex_start + 64]
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        {
            let mut start = at;
            while start > 0 {
                let c = bytes[start - 1];
                if c.is_ascii_alphanumeric() || matches!(c, b'.' | b'/' | b'-' | b'_' | b':') {
                    start -= 1;
                } else {
                    break;
                }
            }
            out.insert(
                text[start..at].to_string(),
                format!("sha256:{}", &text[hex_start..hex_start + 64]),
            );
            i = hex_start + 64;
        } else {
            i = at + 1;
        }
    }
    out
}

/// Not env-gated: the oracle runs `{REPO}/pin-ci-images.sh`, so REPO must
/// stay the repository root (it moved once, in the M5b workspace split).
#[test]
fn oracle_repo_root_has_script() {
    assert!(
        Path::new(REPO).join("pin-ci-images.sh").is_file(),
        "REPO={REPO}"
    );
}

#[tokio::test]
async fn oracle_matches_old_pipeline() {
    // 1. Gate: env + required binaries; else print + return.
    if std::env::var("PIN_CI_ORACLE").is_err() {
        eprintln!("SKIP oracle (set PIN_CI_ORACLE=1)");
        return;
    }
    for bin in ["renovate", "crane"] {
        if !bin_present(bin) {
            eprintln!("SKIP oracle ({bin} not on PATH)");
            return;
        }
    }
    // The old script hardcodes /tmp/renovate.log (append) and reads it back;
    // sandboxed sessions may grant /tmp write-only. Probe read+write; if the
    // temp area is unusable, skip with a remedy hint instead of failing.
    let probe = std::env::temp_dir().join(format!("pin-ci-oracle-probe-{}", std::process::id()));
    let probe_ok = std::fs::write(&probe, "x").is_ok() && std::fs::read_to_string(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    if !probe_ok {
        eprintln!(
            "SKIP oracle (temp dir not readable+writable; grant /tmp read, or symlink /tmp/renovate.log to a readable file)"
        );
        return;
    }

    // 2. Temp dir with the CI file.
    let base = std::env::temp_dir().join(format!("pin-ci-oracle-{}", std::process::id()));
    let dir_old = base.join("old");
    let dir_new = base.join("new");
    for d in [dir_old.join("ci"), dir_new.join("ci")] {
        std::fs::create_dir_all(&d).unwrap();
    }
    let yml_old = dir_old.join("ci/.gitlab-ci.yml");
    let yml_new = dir_new.join("ci/.gitlab-ci.yml");
    std::fs::write(&yml_old, CI_YAML).unwrap();

    // 3. Copy to a second temp dir; run the OLD pipeline as-is on the first.
    std::fs::copy(&yml_old, &yml_new).unwrap();

    // The old pipeline runs renovate, which requires a git repository.
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args([
                "-c",
                "user.email=oracle@pin-ci",
                "-c",
                "user.name=oracle",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&dir_old)
            .output()
            .expect("git ran");
        if !out.status.success() {
            panic!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    };
    git(&["init", "-b", "main"]);
    git(&["add", "ci/.gitlab-ci.yml"]);
    git(&["commit", "-m", "ci"]);

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(format!(
        "cd {} && bash {}/pin-ci-images.sh",
        dir_old.display(),
        REPO
    ));
    // crane treats an UNREADABLE ~/.docker/config.json as a hard error (seen
    // in sandboxed sessions where stat succeeds but open is denied). Point it
    // at an empty config dir when the real one is missing or unreadable; in
    // normal environments open succeeds and user auth is kept.
    if let Some(home) = std::env::var_os("HOME") {
        let real = std::path::PathBuf::from(home).join(".docker/config.json");
        if std::fs::File::open(&real).is_err() {
            let empty = base.join("empty-docker");
            std::fs::create_dir_all(&empty).unwrap();
            cmd.env("DOCKER_CONFIG", &empty);
        }
    }
    let out = cmd.output().expect("old pipeline ran");
    if !out.status.success() {
        panic!(
            "old pipeline failed (status {:?}):\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }

    // 4. Run the NEW tool (pin-only) on the second dir's copy of the
    //    original file.
    let paths = vec![yml_new.clone()];
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
        update: false,
        update_major: false,
        gitlab_host: "gitlab.com".into(),
        ..Default::default()
    };
    let report = pin_files(&paths, &opts, &client_from_env(), &gitlab_dummy())
        .await
        .expect("new tool run");

    // 5. Both produced pins; digests agree per image:tag (see file header).
    let old_text = std::fs::read_to_string(&yml_old).unwrap();
    let old_pins = pinned_pairs(&old_text);
    assert!(
        !old_pins.is_empty(),
        "old pipeline produced no pins:\n{old_text}"
    );
    let new_text = std::fs::read_to_string(&yml_new).unwrap();
    let new_pins = pinned_pairs(&new_text);
    assert!(
        !new_pins.is_empty(),
        "new tool produced no pins:\n{new_text}\nreport: {report:?}"
    );

    let client = client_from_env();
    let mut mismatches = Vec::new();
    for (image, old_digest) in &old_pins {
        let r = match ImageRef::parse(image) {
            Ok(r) => r,
            Err(e) => {
                mismatches.push(format!("{image}: unparseable by new tool ({e})"));
                continue;
            }
        };
        match client.digest(&r).await {
            Ok(d) if &d == old_digest => {}
            Ok(d) => mismatches.push(format!("{image}: old={old_digest} new={d}")),
            Err(e) => mismatches.push(format!("{image}: old={old_digest} new=error({e})")),
        }
    }
    assert!(
        mismatches.is_empty(),
        "digest mismatches vs old pipeline:\n{}",
        mismatches.join("\n")
    );

    // 6. Cleanup.
    let _ = std::fs::remove_dir_all(&dir_old);
    let _ = std::fs::remove_dir_all(&dir_new);
}

// ---------------------------------------------------------------------------
// M2 differential oracle: old pipeline (which bumps tags by default) vs
// `pin-ci --update --update-major`.
//
// Neither side runs an age guard on tag selection: the new tool's
// min_age_days defaults to 0 here, and the old pipeline's config.js
// `minimumReleaseAge: 14d` is scoped to `matchUpdateTypes:
// ['pinDigest','digest']` — its TAG bumps are not age-gated (proven from
// the old pipeline's own log, 2026-11-10 oracle run: renovate accepted
// 12-day-old and 2-day-old tag bumps).
//
// Per original reference position (1-based line + repo in the clean
// fixture) the outputs must agree: tag equal -> digests byte-identical;
// tag differs -> excused ONLY when both tags parse-stable-compatible and
// compare(new, old) == Greater (the documented divergence is VERSIONING
// POLICY, e.g. renovate's custom `ubuntu` versioning treats odd-minor
// interlude releases as unstable where the minimal port does not), in
// which case both digests are sanity-checked with `crane digest` instead
// of cross-equality. Anything else FAILS.
//
// The M4b-1 age guard itself is verified elsewhere: the wiremock e2e
// suite (deterministic) and `live_guarded_update` in tests/live.rs (real
// Hub push timestamps).
// ---------------------------------------------------------------------------

/// M2 fixture (file shared with the live update test): four Docker Hub
/// references, two of them on separate lines with the same repo:tag.
const CI_YAML_M2: &str = include_str!("fixtures/ci-update-clean.yml");

/// `(1-based line, repo) -> (tag, "sha256:<64hex>")` for every
/// `repo:tag@sha256:<64 hex>` in `text`, in file order. Keyed on line + repo
/// so two references to the same repo on different lines stay distinct.
fn positioned_pins(text: &str) -> BTreeMap<(usize, String), (String, String)> {
    let mut out = BTreeMap::new();
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let bytes = line.as_bytes();
        let mut i = 0;
        while let Some(pos) = line[i..].find("@sha256:") {
            let at = i + pos;
            let hex_start = at + 8;
            if hex_start + 64 <= bytes.len()
                && line[hex_start..hex_start + 64]
                    .chars()
                    .all(|c| c.is_ascii_hexdigit())
            {
                let mut start = at;
                while start > 0 {
                    let c = bytes[start - 1];
                    if c.is_ascii_alphanumeric() || matches!(c, b'.' | b'/' | b'-' | b'_' | b':') {
                        start -= 1;
                    } else {
                        break;
                    }
                }
                let token = &line[start..at];
                let (repo, tag) = match token.rfind(':') {
                    Some(c) => (token[..c].to_string(), token[c + 1..].to_string()),
                    None => (token.to_string(), String::new()),
                };
                out.insert(
                    (line_no, repo),
                    (tag, format!("sha256:{}", &line[hex_start..hex_start + 64])),
                );
                i = hex_start + 64;
            } else {
                i = at + 1;
            }
        }
    }
    out
}

/// `crane digest <ref>` — same unreadable-`~/.docker/config.json`
/// workaround as the old-pipeline command (point crane at an empty config
/// dir when the real one is missing or unreadable).
fn crane_digest(ref_name: &str, empty_docker: &Path) -> Result<String, String> {
    let mut cmd = Command::new("crane");
    cmd.args(["digest", ref_name]);
    if let Some(home) = std::env::var_os("HOME") {
        let real = PathBuf::from(home).join(".docker/config.json");
        if std::fs::File::open(&real).is_err() {
            std::fs::create_dir_all(empty_docker).map_err(|e| e.to_string())?;
            cmd.env("DOCKER_CONFIG", empty_docker);
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
async fn m2_oracle_update_matches_old_pipeline() {
    // 1. Gate: env + the four binaries the old pipeline needs; else print
    //    and return.
    if std::env::var("PIN_CI_ORACLE").is_err() {
        eprintln!("SKIP m2 oracle (set PIN_CI_ORACLE=1)");
        return;
    }
    for bin in ["renovate", "crane", "python3", "git"] {
        if !bin_present(bin) {
            eprintln!("SKIP m2 oracle ({bin} not on PATH)");
            return;
        }
    }
    let probe = std::env::temp_dir().join(format!("pin-ci-oracle-m2-probe-{}", std::process::id()));
    let probe_ok = std::fs::write(&probe, "x").is_ok() && std::fs::read_to_string(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    if !probe_ok {
        eprintln!(
            "SKIP m2 oracle (temp dir not readable+writable; set TMPDIR to a writable, readable dir)"
        );
        return;
    }
    // The old script hardcodes /tmp/renovate.log (append, then read back);
    // /tmp may be write-only in sandboxes. Self-heal the symlink to the
    // repo's gitignored .tmp scratch area, then probe read+write.
    let log = PathBuf::from("/tmp/renovate.log");
    if std::fs::read_to_string(&log).is_err()
        || std::fs::OpenOptions::new().append(true).open(&log).is_err()
    {
        let scratch = Path::new(REPO).join(".tmp");
        let _ = std::fs::create_dir_all(&scratch);
        let _ = std::fs::remove_file(&log);
        let _ = std::os::unix::fs::symlink(scratch.join("renovate.log"), &log);
    }
    if std::fs::read_to_string(&log).is_err()
        || std::fs::OpenOptions::new().append(true).open(&log).is_err()
    {
        eprintln!(
            "SKIP m2 oracle (/tmp/renovate.log not readable+writable; symlink it to a readable scratch file)"
        );
        return;
    }

    let base = std::env::temp_dir().join(format!("pin-ci-oracle-m2-{}", std::process::id()));
    let dir_old = base.join("old");
    let dir_new = base.join("new");
    for d in [
        dir_old.join("ci"),
        dir_new.join("ci"),
        base.join("empty-docker"),
    ] {
        std::fs::create_dir_all(&d).unwrap();
    }
    let yml_old = dir_old.join("ci/.gitlab-ci.yml");
    let yml_new = dir_new.join("ci/.gitlab-ci.yml");
    std::fs::write(&yml_old, CI_YAML_M2).unwrap();
    std::fs::write(&yml_new, CI_YAML_M2).unwrap();

    // 2. Old pipeline on a temp git checkout. `pin-ci-images.sh` hardcodes
    //    TARGET_FILE=ci/.gitlab-ci.yml relative to CWD and finds its
    //    python/config via SCRIPT_DIR, so run it with CWD = the temp repo.
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args([
                "-c",
                "user.email=oracle@pin-ci",
                "-c",
                "user.name=oracle",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&dir_old)
            .output()
            .expect("git ran");
        if !out.status.success() {
            panic!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    };
    git(&["init", "-b", "main"]);
    git(&["add", "ci/.gitlab-ci.yml"]);
    git(&["commit", "-m", "ci"]);

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(format!(
        "cd {} && bash {}/pin-ci-images.sh",
        dir_old.display(),
        REPO
    ));
    // crane treats an UNREADABLE ~/.docker/config.json as a hard error (seen
    // in sandboxed sessions where stat succeeds but open is denied). Point
    // it at an empty config dir when the real one is missing or unreadable;
    // in normal environments open succeeds and user auth is kept.
    if let Some(home) = std::env::var_os("HOME") {
        let real = PathBuf::from(home).join(".docker/config.json");
        if std::fs::File::open(&real).is_err() {
            cmd.env("DOCKER_CONFIG", base.join("empty-docker"));
        }
    }
    let out = cmd.output().expect("old pipeline ran");
    if !out.status.success() {
        panic!(
            "old pipeline failed (status {:?}):\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
    let old_text = std::fs::read_to_string(&yml_old).expect("read old output");
    let old_pins = positioned_pins(&old_text);
    assert!(
        !old_pins.is_empty(),
        "old pipeline produced no pins:\n{old_text}"
    );

    // 3. New tool on the second copy of the clean fixture:
    //    --update --update-major.
    let report = pin_files(
        std::slice::from_ref(&yml_new),
        &PinOptions {
            dry_run: false,
            json: false,
            concurrency: 4,
            update: true,
            update_major: true,
            gitlab_host: "gitlab.com".into(),
            ..Default::default()
        },
        &client_from_env(),
        &gitlab_dummy(),
    )
    .await
    .expect("new tool run");
    let new_text = std::fs::read_to_string(&yml_new).expect("read new output");
    let new_pins = positioned_pins(&new_text);
    assert!(
        !new_pins.is_empty(),
        "new tool produced no pins:\n{new_text}\nreport: {report:?}"
    );

    // 4. Compare per original reference position (1-based line + repo).
    assert_eq!(
        old_pins.keys().collect::<Vec<_>>(),
        new_pins.keys().collect::<Vec<_>>(),
        "reference positions differ between old pipeline and new tool\nold: {old_text}\nnew: {new_text}"
    );
    let empty_docker = base.join("empty-docker");
    let mut mismatches = Vec::new();
    let mut notes = Vec::new();
    for ((line, repo), (old_tag, old_digest)) in &old_pins {
        // Keys were asserted equal above; build an owned key for the index
        // (line and repo are borrowed from old_pins).
        let (new_tag, new_digest) = &new_pins[&(*line, repo.clone())];
        if old_tag == new_tag {
            if old_digest != new_digest {
                mismatches.push(format!(
                    "{repo} (line {line}): tags equal ({old_tag}) but digests differ: old={old_digest} new={new_digest}"
                ));
            }
            continue;
        }
        // Tag differs: allowed ONLY as the documented divergence — both
        // tags parse-stable-compatible and new strictly greater (the
        // versioning-policy gap, e.g. renovate's `ubuntu` versioning
        // treats odd-minor interludes as unstable). In that case both
        // digests are sanity-checked with `crane digest` instead of
        // cross-equality; anything else FAILS.
        let excused = match (versioning::parse(old_tag), versioning::parse(new_tag)) {
            (Some(v_old), Some(v_new)) => {
                v_old.is_stable()
                    && v_new.is_stable()
                    && v_old.is_compatible(&v_new)
                    && versioning::compare(new_tag, old_tag) == Some(std::cmp::Ordering::Greater)
            }
            _ => false,
        };
        if !excused {
            mismatches.push(format!(
                "{repo} (line {line}): tags differ beyond the documented divergence rule: old={repo}:{old_tag}@{old_digest} new={repo}:{new_tag}@{new_digest}"
            ));
            continue;
        }
        for (tag, digest, side) in [(old_tag, old_digest, "old"), (new_tag, new_digest, "new")] {
            match crane_digest(&format!("{repo}:{tag}"), &empty_docker) {
                Ok(cd) if cd == *digest => {}
                Ok(cd) => mismatches.push(format!(
                    "{repo} (line {line}): {side} digest {digest} != crane {cd} for {repo}:{tag}"
                )),
                Err(e) => mismatches.push(format!("{repo} (line {line}): crane check: {e}")),
            }
        }
        notes.push(format!(
            "documented divergence {repo} (line {line}): old={repo}:{old_tag}@{old_digest} new={repo}:{new_tag}@{new_digest} (versioning-policy divergence, e.g. ubuntu odd-minor interlude; both digests crane-verified)"
        ));
    }
    for n in &notes {
        eprintln!("{n}");
    }
    assert!(
        mismatches.is_empty(),
        "M2 oracle mismatches:\n{}",
        mismatches.join("\n")
    );

    // 5. Cleanup.
    let _ = std::fs::remove_dir_all(&base);
}
