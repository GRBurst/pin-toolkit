//! Opt-in live smoke of the real binary (real Docker Hub, T30). Gate:
//! `PIN_CI_LIVE=1 cargo test -p pin-toolkit --test live`. Without the env
//! var it prints a notice and passes without doing anything.
#![cfg(feature = "gitlab-ci")]

mod support;

use std::path::Path;
use std::process::{Command, Output};

/// `pin-toolkit args…` in `cwd` with an empty environment (`HOME` = `cwd`,
/// so no machine credential or `PIN_TOOLKIT_*` setting takes part).
fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pin-toolkit"))
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("HOME", cwd)
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// `image: alpine:3.20@sha256:<64 hex>\n` — C1 pins inline, without a
/// trailing comment (PR-M8).
fn is_pinned_alpine(file: &str) -> bool {
    file.strip_prefix("image: alpine:3.20@sha256:")
        .and_then(|rest| rest.strip_suffix('\n'))
        .is_some_and(|hex| {
            hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
}

#[test]
fn live_pins_public_image() {
    if std::env::var("PIN_CI_LIVE").is_err() {
        eprintln!("SKIP live (set PIN_CI_LIVE=1)");
        return;
    }
    let root = support::repo("live-pin");
    support::write(&root, ".gitlab-ci.yml", "image: alpine:3.20\n");
    let out = run(&root, &[]);
    let stdout = text(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {}",
        text(&out.stderr)
    );
    let file = std::fs::read_to_string(root.join(".gitlab-ci.yml")).unwrap();
    assert!(is_pinned_alpine(&file), "{file:?}");
    let row = stdout.lines().next().unwrap_or_default();
    assert!(
        row.starts_with(".gitlab-ci.yml:1  gitlab-ci:image  pinned  alpine:3.20 -> ")
            && row.ends_with("  # alpine:3.20"),
        "{stdout}"
    );

    let out = run(&root, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--check after the pin: {}{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(root.join(".gitlab-ci.yml")).unwrap(),
        file
    );
}
