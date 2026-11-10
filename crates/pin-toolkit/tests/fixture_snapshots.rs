//! Toolkit snapshots over the M5a fixtures (spec §9 M5d, T-D24): for every
//! fixture and mode, the human and the JSON report of `pin-toolkit
//! gitlab-ci`, and P-same-bytes: the toolkit writes exactly the bytes the
//! pin-gitlab-ci driver writes (its `__file` snapshots stay there).
//!
//! Registries and GitLab are the fakes of the pin-gitlab-ci harness.
//! Accept with `INSTA_UPDATE=always`, review with `git diff`.
#![cfg(feature = "gitlab-ci")]

#[path = "../../pin-gitlab-ci/tests/common/mod.rs"]
mod common;
mod support;

use std::path::Path;

use common::*;
use support::{fakes, run_tuned};

/// `pin-toolkit gitlab-ci [--json] --concurrency 4 [--update] {name}` in
/// the scratch dir of `file` (made a git root); returns stdout.
async fn toolkit(f: &Fakes, file: &Path, mode: Mode, json: bool) -> String {
    let dir = file.parent().unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let name = file.file_name().unwrap().to_str().unwrap();
    let mut argv = vec!["pin-toolkit", "gitlab-ci", "--concurrency", "4"];
    if json {
        argv.push("--json");
    }
    if matches!(mode, Mode::Update) {
        argv.push("--update");
    }
    argv.push(name);
    let out = run_tuned(dir, &argv, &[], fakes(f.registry.uri(), f.gitlab.uri())).await;
    assert_eq!(out.stderr, "", "{name} ({json}): no errors expected");
    out.stdout
}

#[tokio::test]
async fn toolkit_snapshots_over_fixtures() {
    let f = start_fakes().await;
    for fx in fixtures() {
        for m in modes(&fx.name) {
            let (human, json, driver) = (
                scratch_copy(&fx, m, "toolkit-human"),
                scratch_copy(&fx, m, "toolkit-json"),
                scratch_copy(&fx, m, "toolkit-driver"),
            );
            let id = format!("{}__{}", fx.stem, m.tag());
            let stdout = toolkit(&f, &human, m, false).await;
            insta::assert_snapshot!(format!("{id}__human"), redact(&stdout));
            let stdout = toolkit(&f, &json, m, true).await;
            insta::assert_snapshot!(format!("{id}__json"), redact(&stdout));
            pin_once(&f, &driver, m).await;
            let want = std::fs::read(&driver).unwrap();
            for p in [&human, &json] {
                assert!(
                    std::fs::read(p).unwrap() == want,
                    "{id}: toolkit bytes differ from driver bytes (P-same-bytes)"
                );
            }
            for p in [&human, &json, &driver] {
                std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
            }
        }
    }
}
