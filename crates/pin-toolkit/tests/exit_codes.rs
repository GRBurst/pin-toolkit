//! The exit-code table (spec §6.5, §9 M5d gate): 0/1/2, `--check`, fatal
//! cases, one `.tmp/` repo per row, against the pin-gitlab-ci fakes.
//! P-check-pure (`--check`/`--dry-run` write nothing) after every such row;
//! P-exit recomputed from the printed JSON where a row uses `--json`.
#![cfg(feature = "gitlab-ci")]

#[path = "../../pin-gitlab-ci/tests/common/mod.rs"]
mod common;
mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::{Fakes, start_fakes};
use pin_toolkit::Output;
use support::{fakes, repo, run_tuned, write};

const NEEDS_PIN: &str = "image: alpine:3.20\n";

/// Every file under `root` except `.git/`, with its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != ".git") {
                    dirs.push(p);
                }
            } else {
                out.insert(p.clone(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

/// P-exit from a JSON report: 1 if anything is unresolved, or with `check`
/// if anything would be pinned or updated; else 0 (file errors are not in
/// the JSON until T28).
fn p_exit(json: &str, check: bool) -> u8 {
    let v: serde_json::Value = serde_json::from_str(json).expect("stdout is one JSON report");
    let n = |k: &str| v["summary"][k].as_u64().expect("summary count");
    u8::from(n("unresolved") > 0 || (check && n("pinned") + n("updated") > 0))
}

/// Row `n`: a fresh repo with `files`, then `pin-toolkit {args}` with the
/// env map `vars`; returns the repo and the output.
async fn row(
    f: &Fakes,
    n: &str,
    files: &[(&str, &str)],
    args: &[&str],
    vars: &[(&str, &str)],
) -> (PathBuf, Output) {
    let root = repo(&format!("exit-{n}"));
    for (rel, text) in files {
        write(&root, rel, text);
    }
    let out = run(f, &root, args, vars).await;
    (root, out)
}

/// `pin-toolkit {args}` in `root`, resolvers on the fakes.
async fn run(f: &Fakes, root: &Path, args: &[&str], vars: &[(&str, &str)]) -> Output {
    let argv: Vec<&str> = std::iter::once("pin-toolkit")
        .chain(args.iter().copied())
        .collect();
    run_tuned(root, &argv, vars, fakes(f.registry.uri(), f.gitlab.uri())).await
}

/// Row `n` with `--check`/`--dry-run`: the same row, and P-check-pure.
async fn pure_row(f: &Fakes, n: &str, files: &[(&str, &str)], args: &[&str]) -> (PathBuf, Output) {
    let root = repo(&format!("exit-{n}"));
    for (rel, text) in files {
        write(&root, rel, text);
    }
    let before = snapshot(&root);
    let out = run(f, &root, args, &[]).await;
    assert!(
        snapshot(&root) == before,
        "row {n}: P-check-pure (bytes unchanged)"
    );
    (root, out)
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

fn code(n: &str, out: &Output, want: u8) {
    assert_eq!(
        out.code, want,
        "row {n}: exit code\nstdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
}

#[tokio::test]
async fn exit_code_table() {
    let f = start_fakes().await;
    let pin = [(".gitlab-ci.yml", NEEDS_PIN)];

    // 1: needs a pin → 0, file changed.
    let (root, out) = row(&f, "1", &pin, &[], &[]).await;
    code("1", &out, 0);
    assert_ne!(
        read(&root, ".gitlab-ci.yml"),
        NEEDS_PIN,
        "row 1: file changed"
    );

    // 2: same with --check → 1, bytes unchanged.
    let (_, out) = pure_row(&f, "2", &pin, &["--check"]).await;
    code("2", &out, 1);

    // 3: after row 1, --check → 0 (idempotent).
    let (root, out) = row(&f, "3", &pin, &[], &[]).await;
    code("3 (pin)", &out, 0);
    let before = snapshot(&root);
    let out = run(&f, &root, &["--check"], &[]).await;
    code("3", &out, 0);
    assert!(snapshot(&root) == before, "row 3: P-check-pure");

    // 4: a reference the fake registry 404s → 1, row unresolved.
    let missing = [(".gitlab-ci.yml", "image: missing:1\n")];
    let (_, out) = row(&f, "4", &missing, &[], &[]).await;
    code("4", &out, 1);
    assert!(
        out.stdout.starts_with(".gitlab-ci.yml:1  unresolved ("),
        "row 4: {}",
        out.stdout
    );

    // 5: same with --check --json → 1, P-exit from the JSON.
    let (_, out) = pure_row(&f, "5", &missing, &["--check", "--json"]).await;
    code("5", &out, 1);
    assert_eq!(
        p_exit(&out.stdout, true),
        out.code,
        "row 5: P-exit from JSON"
    );

    // 6: one invalid YAML file among two → 2, the good file pinned.
    let bad = [
        (".gitlab-ci.yml", NEEDS_PIN),
        ("bad.gitlab-ci.yml", "a: 1\nb: c: d\n"),
    ];
    let (root, out) = row(&f, "6", &bad, &[], &[]).await;
    code("6", &out, 2);
    assert_ne!(
        read(&root, ".gitlab-ci.yml"),
        NEEDS_PIN,
        "row 6: good file pinned"
    );
    assert_eq!(read(&root, "bad.gitlab-ci.yml"), "a: 1\nb: c: d\n");
    assert_eq!(
        out.stderr,
        "pin-toolkit: bad.gitlab-ci.yml:2: invalid YAML: mapping values are not allowed in this context\n",
        "row 6"
    );

    // 7: an explicit missing file → 2.
    let (_, out) = row(&f, "7", &[], &["gitlab-ci", "missing.yml"], &[]).await;
    code("7", &out, 2);
    assert!(
        out.stderr.starts_with("pin-toolkit: missing.yml: "),
        "row 7: {}",
        out.stderr
    );

    // 8: usage error → 2, stdout empty.
    let (_, out) = row(&f, "8", &pin, &["--bogus"], &[]).await;
    code("8", &out, 2);
    assert_eq!(out.stdout, "", "row 8");

    // 9: `[hosts]` in the repo file → 2, nothing written.
    let hosts = [
        (".gitlab-ci.yml", NEEDS_PIN),
        (
            ".pin-toolkit.toml",
            "version = 1\n[hosts.\"gitlab.example.com\"]\nkind = \"gitlab\"\n",
        ),
    ];
    let (root, out) = row(&f, "9", &hosts, &[], &[]).await;
    code("9", &out, 2);
    assert_eq!(
        read(&root, ".gitlab-ci.yml"),
        NEEDS_PIN,
        "row 9: nothing written"
    );
    assert_eq!(out.stdout, "", "row 9");

    // 10: an explicit file no plugin claims → 2.
    let readme = [("docs/readme.yml", NEEDS_PIN)];
    let (_, out) = row(&f, "10", &readme, &["docs/readme.yml"], &[]).await;
    code("10", &out, 2);
    assert_eq!(
        out.stderr,
        "pin-toolkit: no plugin claims docs/readme.yml (choose one: pin-toolkit <plugin> docs/readme.yml)\n",
        "row 10"
    );

    // 11, 12: a plugin not in this build → 2.
    for (n, args) in [("11", &["helm"][..]), ("12", &["--only", "helm"][..])] {
        let (_, out) = row(&f, n, &pin, args, &[]).await;
        code(n, &out, 2);
        assert_eq!(
            out.stderr, "pin-toolkit: plugin helm not in this build\n",
            "row {n}"
        );
    }

    // 13: needs a pin, --dry-run → 0, bytes unchanged.
    let (_, out) = pure_row(&f, "13", &pin, &["--dry-run"]).await;
    code("13", &out, 0);

    // 14: templated reference only, --check → 0, skipped, no request.
    let quiet = start_fakes().await;
    let templated = [(".gitlab-ci.yml", "image: $CI_REGISTRY_IMAGE:1\n")];
    let (_, out) = pure_row(&quiet, "14", &templated, &["--check"]).await;
    code("14", &out, 0);
    assert_eq!(
        out.stdout,
        ".gitlab-ci.yml:1  skipped (unparseable reference)  $CI_REGISTRY_IMAGE:1 -> -  # $CI_REGISTRY_IMAGE:1\n\
         pinned: 0, ok: 0, updated: 0, skipped: 1, unresolved: 0\n",
        "row 14"
    );
    for server in [&quiet.registry, &quiet.gitlab] {
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "row 14: no request"
        );
    }

    // 15: an invalid glob in the repo file → 2.
    let glob = [
        (".gitlab-ci.yml", NEEDS_PIN),
        (
            ".pin-toolkit.toml",
            "version = 1\n[plugins.gitlab-ci]\nfiles = [\"[\"]\n",
        ),
    ];
    let (_, out) = row(&f, "15", &glob, &[], &[]).await;
    code("15", &out, 2);
    assert!(
        out.stderr
            .starts_with("pin-toolkit: invalid glob `[` in plugins.gitlab-ci.files: "),
        "row 15: {}",
        out.stderr
    );

    // 16: an unknown PIN_TOOLKIT_* variable only warns (PR-M3).
    let (root, out) = row(&f, "16", &pin, &[], &[("PIN_TOOLKIT_VERSION", "0.2")]).await;
    code("16", &out, 0);
    assert_ne!(
        read(&root, ".gitlab-ci.yml"),
        NEEDS_PIN,
        "row 16: file changed"
    );
    assert_eq!(
        out.stderr, "pin-toolkit: ignoring unknown environment variable PIN_TOOLKIT_VERSION\n",
        "row 16"
    );
}
