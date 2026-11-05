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
use std::process::Command;

use pin_ci::{ImageRef, PinOptions, client_from_env, pin_files};

/// Repo root at compile time; the old pipeline runs as-is from here.
const REPO: &str = env!("CARGO_MANIFEST_DIR");

const CI_YAML: &str = "\
image: alpine:3.20
build:
  image: ubuntu:24.04
  services:
    - docker:24
";

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
    };
    let report = pin_files(&paths, &opts, &client_from_env())
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
