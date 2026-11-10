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
            ".gitlab-ci.yml:1  pinned  alpine:3.20 -> {pinned}  # alpine:3.20\n\
             pinned: 1, ok: 0, updated: 0, skipped: 0, unresolved: 0\n"
        )
    );
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
