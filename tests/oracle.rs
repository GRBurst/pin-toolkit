//! Differential oracle test against the old pipeline (owner's required gate).
//!
//! Gate: run with `PIN_CI_ORACLE=1 cargo test --test oracle` and with both
//! `renovate` and `crane` on `PATH`. Otherwise the test prints a notice and
//! passes without doing anything.
use std::collections::BTreeSet;
use std::process::Command;

use pin_ci::{PinOptions, client_from_env, pin_files};

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

/// Extract all `[\w./-]+@sha256:[0-9a-f]{64}` occurrences in `text`.
/// Manual scan (no regex dependency): find each `@sha256:`, verify 64 hex
/// digits follow, then extend backwards over `[\w./-]`.
fn digest_occurrences(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(pos) = text[i..].find("@sha256:") {
        let at = i + pos;
        let hex_start = at + 7;
        if hex_start + 64 <= bytes.len()
            && text[hex_start..hex_start + 64]
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        {
            let mut start = at;
            while start > 0 {
                let c = bytes[start - 1];
                if c.is_ascii_alphanumeric() || c == b'.' || c == b'/' || c == b'-' || c == b'_' {
                    start -= 1;
                } else {
                    break;
                }
            }
            out.insert(text[start..hex_start + 64].to_string());
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

    // 2. Temp dir with the CI file (old enough Docker Hub tags that the old
    // tool's 14-day minimumReleaseAge cannot skew it).
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
    let cmd = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "cd {} && bash {}/pin-ci-images.sh",
            dir_old.display(),
            REPO
        ))
        .output()
        .expect("old pipeline ran");
    if !cmd.status.success() {
        panic!(
            "old pipeline failed (status {:?}):\nstdout:\n{}\nstderr:\n{}",
            cmd.status,
            String::from_utf8_lossy(&cmd.stdout),
            String::from_utf8_lossy(&cmd.stderr),
        );
    }

    // 4. Run the NEW tool on the second dir's file.
    let paths = vec![yml_new.clone()];
    let opts = PinOptions {
        dry_run: false,
        json: false,
        concurrency: 4,
    };
    let report = pin_files(&paths, &opts, &client_from_env())
        .await
        .expect("new tool run");

    // 5. Equal non-empty sets of pinned `name@sha256:...` occurrences.
    let old_text = std::fs::read_to_string(&yml_old).unwrap();
    let new_text = std::fs::read_to_string(&yml_new).unwrap();
    let old_pins = digest_occurrences(&old_text);
    let new_pins = digest_occurrences(&new_text);
    assert!(
        !old_pins.is_empty(),
        "old pipeline produced no pins:\n{old_text}"
    );
    assert!(
        !new_pins.is_empty(),
        "new tool produced no pins: {report:?}"
    );
    assert_eq!(old_pins, new_pins);

    // 6. Cleanup.
    let _ = std::fs::remove_dir_all(&dir_old);
    let _ = std::fs::remove_dir_all(&dir_new);
}
