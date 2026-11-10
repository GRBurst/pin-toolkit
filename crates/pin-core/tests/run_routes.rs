//! `run::run` over routes (spec §6.3, M5d T12): one pass, rows carry the
//! display path, per-file errors are recorded under the plugin and later
//! files continue, follow-ups are collected once.

use std::path::{Path, PathBuf};

use pin_core::edit::{Splice, apply};
use pin_core::plugin::{
    Finding, KindSet, KindSpec, Plugin, PluginMeta, ScanError, Selection, Source,
};
use pin_core::report::{ErrorEntry, Status};
use pin_core::run::{Ctx, ResolverEnv, Route, RunOptions, run};

static KINDS: [KindSpec; 1] = [KindSpec {
    id: "x",
    default_on: true,
    help: "",
}];

/// Every file yields `findings` (one target each), or `scan_error`.
struct Fake {
    findings: Vec<Finding>,
    scan_error: Option<ScanError>,
}

impl PluginMeta for Fake {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn kinds(&self) -> &'static [KindSpec] {
        &KINDS
    }
    fn default_files(&self) -> &'static [&'static str] {
        &[]
    }
}

impl Plugin for Fake {
    type Target = usize;

    fn scan(&self, _path: &Path, _text: &str, _on: &KindSet) -> Result<Vec<usize>, ScanError> {
        match &self.scan_error {
            Some(e) => Err(e.clone()),
            None => Ok((0..self.findings.len()).collect()),
        }
    }

    async fn resolve(&self, _ctx: &Ctx, _src: &Source<'_>, t: &usize) -> Finding {
        self.findings[*t].clone()
    }
}

const TEXT: &str = "image: abcdefgh\nnext: line\n";

fn pinned(follow_up: Option<&str>) -> Finding {
    Finding {
        line: 0,
        kind: "x",
        reference: "abcdefgh".into(),
        old: "abcdefgh".into(),
        new: Some("pinned".into()),
        status: Status::Pinned,
        reason: None,
        edits: vec![Splice::replace(7, 15, "pinned")],
        follow_up: follow_up.map(String::from),
    }
}

fn fake(findings: Vec<Finding>) -> Fake {
    Fake {
        findings,
        scan_error: None,
    }
}

fn ctx() -> Ctx {
    Ctx::new(Default::default(), 0, ResolverEnv::default())
}

fn opts() -> RunOptions {
    RunOptions {
        dry_run: false,
        check: false,
        concurrency: 4,
    }
}

/// A fresh `.tmp/run-routes-{test}-{pid}/` with `files` (name, text).
fn scratch(test: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = PathBuf::from(format!(".tmp/run-routes-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn route<'a>(p: &'a Fake, sel: &'a Selection, path: PathBuf, display: &str) -> Route<'a> {
    Route {
        plugin: p,
        sel,
        path,
        display: display.into(),
    }
}

/// §6.3: a file that cannot be read is an error entry under its plugin; the
/// next file is still pinned and written; exit 2.
#[tokio::test]
async fn run_continues_after_file_error() {
    let dir = scratch("continue", &[("b.yml", TEXT)]);
    let p = fake(vec![pinned(None)]);
    let sel: Selection = KindSet::defaults(&KINDS).into();
    let a = dir.join("a.yml");
    let read_msg = std::fs::read_to_string(&a).unwrap_err().to_string();
    let routes = [
        route(&p, &sel, a, "a-display"),
        route(&p, &sel, dir.join("b.yml"), "b-display"),
    ];
    let report = run(&routes, &ctx(), opts()).await;

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].file, "b-display");
    assert_eq!(report.results[0].status, Status::Pinned);
    assert_eq!(
        std::fs::read_to_string(dir.join("b.yml")).unwrap(),
        apply(TEXT, &p.findings[0].edits)
    );
    assert_eq!(
        report.plugins["fake"].errors,
        [ErrorEntry {
            file: "a-display".into(),
            line: None,
            message: read_msg,
        }]
    );
    assert_eq!(report.plugins["fake"].summary.pinned, 1);
    assert_eq!(report.exit_code(false), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// §6.9: a parse error keeps its line; the message has no path; the file is
/// untouched.
#[tokio::test]
async fn run_parse_error_has_line() {
    let dir = scratch("parse", &[("a.yml", TEXT)]);
    let p = Fake {
        findings: vec![],
        scan_error: Some(ScanError {
            line: Some(3),
            msg: "invalid YAML: x".into(),
        }),
    };
    let sel: Selection = KindSet::defaults(&KINDS).into();
    let routes = [route(&p, &sel, dir.join("a.yml"), "a-display")];
    let report = run(&routes, &ctx(), opts()).await;

    assert!(report.results.is_empty());
    assert_eq!(
        report.plugins["fake"].errors,
        [ErrorEntry {
            file: "a-display".into(),
            line: Some(3),
            message: "invalid YAML: x".into(),
        }]
    );
    assert_eq!(std::fs::read_to_string(dir.join("a.yml")).unwrap(), TEXT);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Rows carry the route's display path (not the I/O path), in route order;
/// a plugin with routed files has an entry.
#[tokio::test]
async fn run_rows_use_display_path() {
    let dir = scratch("display", &[("x.yml", TEXT), ("y.yml", TEXT)]);
    let p = fake(vec![pinned(None)]);
    let sel: Selection = KindSet::defaults(&KINDS).into();
    let routes = [
        route(&p, &sel, dir.join("x.yml"), "shown/x.yml"),
        route(&p, &sel, dir.join("y.yml"), "../y.yml"),
    ];
    let report = run(&routes, &ctx(), opts()).await;

    assert_eq!(
        report
            .results
            .iter()
            .map(|r| r.file.as_str())
            .collect::<Vec<_>>(),
        ["shown/x.yml", "../y.yml"]
    );
    assert_eq!(
        report
            .plugins
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["fake"]
    );
    assert_eq!(report.exit_code(false), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same follow-up from two files is reported once.
#[tokio::test]
async fn run_collects_follow_ups() {
    let dir = scratch("follow", &[("a.yml", TEXT), ("b.yml", TEXT)]);
    let p = fake(vec![pinned(Some("helm dependency update charts/app"))]);
    let sel: Selection = KindSet::defaults(&KINDS).into();
    let routes = [
        route(&p, &sel, dir.join("a.yml"), "a.yml"),
        route(&p, &sel, dir.join("b.yml"), "b.yml"),
    ];
    let report = run(&routes, &ctx(), opts()).await;

    assert_eq!(report.results.len(), 2);
    assert_eq!(report.follow_ups, ["helm dependency update charts/app"]);
    let _ = std::fs::remove_dir_all(&dir);
}
