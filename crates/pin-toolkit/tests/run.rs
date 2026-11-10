//! The one-pass run through `execute` (spec §6.3, §6.7): walk, route, pin,
//! report, against the fakes of the pin-gitlab-ci harness.
#![cfg(feature = "gitlab-ci")]

#[path = "../../pin-gitlab-ci/tests/common/mod.rs"]
mod common;
mod support;

use common::{Fakes, fake_hex, start_fakes};
use support::{fakes, repo, run_tuned, write};

/// The resolvers of a run, pointed at `f`.
fn tune(f: &Fakes) -> impl FnOnce(&mut pin_core::run::ResolverEnv) {
    fakes(f.registry.uri(), f.gitlab.uri())
}

#[tokio::test]
async fn pins_a_repo_end_to_end() {
    let f = start_fakes().await;
    let root = repo("run-e2e");
    write(&root, ".gitlab-ci.yml", "image: alpine:3.20\n");
    let out = run_tuned(&root, &["pin-toolkit"], &[], tune(&f)).await;
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "");
    let pinned = format!("alpine:3.20@sha256:{}", fake_hex("library/alpine:3.20", 64));
    // C1 pins inline, without a trailing comment (PR-M8).
    assert_eq!(
        std::fs::read_to_string(root.join(".gitlab-ci.yml")).unwrap(),
        format!("image: {pinned}\n")
    );
    assert_eq!(
        out.stdout,
        format!(
            ".gitlab-ci.yml:1  gitlab-ci:image  pinned  alpine:3.20 -> {pinned}  # alpine:3.20\n\
             pinned: 1, ok: 0, updated: 0, skipped: 0, unresolved: 0\n"
        )
    );
}

/// Review-2 M1: `--check ../other` (a sibling repo outside the git root)
/// checks that repo's files instead of being a silent no-op, with and
/// without a subcommand.
#[tokio::test]
async fn explicit_dir_outside_the_git_root() {
    let f = start_fakes().await;
    let root = repo("run-outside-repo");
    let other = repo("run-outside-other");
    write(&other, ".gitlab-ci.yml", "image: alpine:3.20\n");
    let dir = format!("../{}", other.file_name().unwrap().to_str().unwrap());
    for argv in [
        vec!["pin-toolkit", "--check", &dir],
        vec!["pin-toolkit", "gitlab-ci", "--check", &dir],
    ] {
        let out = run_tuned(&root, &argv, &[], tune(&f)).await;
        assert_eq!(out.code, 1, "{argv:?}: {}{}", out.stdout, out.stderr);
        let row = out.stdout.lines().next().unwrap_or_default();
        assert!(
            row.starts_with(&format!("{dir}/.gitlab-ci.yml:1  ")) && row.contains("  pinned  "),
            "{argv:?}: {}",
            out.stdout
        );
    }
    assert_eq!(
        std::fs::read_to_string(other.join(".gitlab-ci.yml")).unwrap(),
        "image: alpine:3.20\n"
    );
}

/// Review-2 L3: an unreadable directory is a per-file error (exit 2, the
/// other files still run) under the first enabled plugin; an invalid
/// `.gitignore` line only warns (exit unaffected).
#[cfg(unix)]
#[tokio::test]
async fn walk_errors_exit_2_ignore_file_lines_warn() {
    use std::os::unix::fs::PermissionsExt;
    let f = start_fakes().await;
    let root = repo("run-walk-errors");
    write(&root, ".gitlab-ci.yml", "image: alpine:3.20\n");
    write(&root, ".gitignore", "[z-a]\n");
    let warning = "pin-toolkit: warning: .gitignore:1: error parsing glob '[z-a]': invalid range; 'z' > 'a'\n";
    let out = run_tuned(&root, &["pin-toolkit", "--dry-run"], &[], tune(&f)).await;
    assert_eq!(
        (out.code, out.stderr.as_str()),
        (0, warning),
        "{}",
        out.stdout
    );

    write(&root, "locked/x.gitlab-ci.yml", "image: alpine:3.20\n");
    let locked = root.join("locked");
    let mode = |m: u32| std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(m));
    mode(0o000).unwrap();
    if std::fs::read_dir(&locked).is_ok() {
        mode(0o755).unwrap();
        return; // permissions do not apply (e.g. running as root)
    }
    for argv in [
        &["pin-toolkit", "--dry-run"][..],
        &["pin-toolkit", "gitlab-ci", "--dry-run"],
    ] {
        let out = run_tuned(&root, argv, &[], tune(&f)).await;
        assert_eq!(
            (out.code, out.stderr.clone()),
            (
                2,
                format!("{warning}pin-toolkit: locked: Permission denied (os error 13)\n")
            ),
            "{argv:?}"
        );
        assert!(
            out.stdout.starts_with(".gitlab-ci.yml:1  ") && out.stdout.contains("  pinned  "),
            "{argv:?}: the readable file still ran: {}",
            out.stdout
        );
    }
    mode(0o755).unwrap();
}

#[tokio::test]
async fn same_image_in_two_files_is_one_lookup() {
    let f = start_fakes().await;
    let root = repo("run-memo");
    write(&root, ".gitlab-ci.yml", "image: alpine:3.20\n");
    write(&root, "ci/x.gitlab-ci.yml", "image: alpine:3.20\n");
    let out = run_tuned(&root, &["pin-toolkit"], &[], tune(&f)).await;
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .ends_with("pinned: 2, ok: 0, updated: 0, skipped: 0, unresolved: 0\n"),
        "{}",
        out.stdout
    );
    let manifest = "/v2/library/alpine/manifests/3.20";
    let hits: Vec<String> = f
        .registry
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == manifest)
        .map(|r| r.method.to_string())
        .collect();
    assert_eq!(hits, ["HEAD"], "one manifest lookup for two files (P-memo)");
}
