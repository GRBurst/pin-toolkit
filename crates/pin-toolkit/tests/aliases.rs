//! argv[0] dispatch through the real binary (T24, D33): `pin-ci` and
//! `pin-<plugin>` are symlinks to `pin-toolkit`; the alias inserts the plugin
//! subcommand. Templated targets only, so no resolver is built (no network).
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A symlink `.tmp/alias-{tag}-{pid}/bin/{alias}` → the built `pin-toolkit`.
fn link(tag: &str, alias: &str) -> PathBuf {
    let bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!(".tmp/alias-{tag}-{}/bin", std::process::id()));
    let _ = std::fs::remove_dir_all(&bin);
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join(alias);
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_pin-toolkit"), &exe).unwrap();
    exe
}

/// A repo with `notes.yml` (matched by no gitlab-ci glob) holding one
/// templated image (skipped without a lookup).
fn notes_repo(tag: &str) -> PathBuf {
    let root = support::repo(&format!("alias-repo-{tag}"));
    support::write(&root, "notes.yml", "image: $CI_REGISTRY_IMAGE:1\n");
    root
}

/// Run `exe args…` in `cwd` with an empty environment (`HOME` = `cwd`).
fn run(exe: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(exe)
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

#[cfg(feature = "gitlab-ci")]
fn assert_runs_gitlab_ci(alias: &str) {
    let root = notes_repo(alias);
    let out = run(&link(alias, alias), &root, &["--dry-run", "notes.yml"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).starts_with("notes.yml:1  "),
        "{}",
        text(&out.stdout)
    );
}

#[cfg(feature = "gitlab-ci")]
#[test]
fn pin_ci_alias_is_gitlab_ci_subcommand() {
    assert_runs_gitlab_ci("pin-ci");
    // control: without the alias, the file is claimed by no plugin
    let root = notes_repo("control");
    let out = run(
        Path::new(env!("CARGO_BIN_EXE_pin-toolkit")),
        &root,
        &["--dry-run", "notes.yml"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stdout).is_empty());
    assert!(
        text(&out.stderr).starts_with("pin-toolkit: no plugin claims notes.yml "),
        "{}",
        text(&out.stderr)
    );
}

#[cfg(feature = "gitlab-ci")]
#[test]
fn pin_gitlab_ci_alias() {
    assert_runs_gitlab_ci("pin-gitlab-ci");
}

#[test]
fn pin_helm_alias_not_in_build() {
    let root = notes_repo("pin-helm");
    let out = run(&link("pin-helm", "pin-helm"), &root, &["--dry-run"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stdout).is_empty());
    assert_eq!(
        text(&out.stderr),
        "pin-toolkit: plugin helm not in this build\n"
    );
}
