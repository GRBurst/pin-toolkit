//! `run::pin_file` with a fake plugin (M5c C6): bounded join, row order,
//! overlap rule, write rules and per-file errors.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pin_core::edit::{Splice, apply};
use pin_core::plugin::{Finding, KindSet, KindSpec, Plugin, PluginMeta, ScanError};
use pin_core::report::Status;
use pin_core::run::{Ctx, FileError, ResolverEnv, RunOptions, pin_file};

static KINDS: [KindSpec; 1] = [KindSpec {
    id: "x",
    default_on: true,
    help: "",
}];

struct Fake {
    findings: Vec<Finding>,
    scan_error: Option<ScanError>,
    in_flight: Cell<usize>,
    max_in_flight: Cell<usize>,
}

impl Fake {
    fn new(findings: Vec<Finding>) -> Self {
        Self {
            findings,
            scan_error: None,
            in_flight: Cell::new(0),
            max_in_flight: Cell::new(0),
        }
    }
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

    fn scan(&self, _path: &Path, _text: &str, on: &KindSet) -> Result<Vec<usize>, ScanError> {
        if let Some(e) = &self.scan_error {
            return Err(e.clone());
        }
        Ok((0..self.findings.len())
            .filter(|_| on.contains("x"))
            .collect())
    }

    async fn resolve(&self, _ctx: &Ctx, _text: &str, t: &usize) -> Finding {
        self.in_flight.set(self.in_flight.get() + 1);
        self.max_in_flight
            .set(self.max_in_flight.get().max(self.in_flight.get()));
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        self.in_flight.set(self.in_flight.get() - 1);
        self.findings[*t].clone()
    }
}

fn finding(line: usize, status: Status, edits: Vec<Splice>) -> Finding {
    Finding {
        line,
        kind: "x",
        reference: format!("ref{line}"),
        old: "old".into(),
        new: (!edits.is_empty()).then(|| "new".to_string()),
        status,
        reason: None,
        edits,
        follow_up: None,
    }
}

fn ctx() -> Ctx {
    Ctx::new(Default::default(), 0, ResolverEnv::default())
}

fn opts(concurrency: usize) -> RunOptions {
    RunOptions {
        dry_run: false,
        check: false,
        concurrency,
    }
}

fn on() -> KindSet {
    KindSet::defaults(&KINDS)
}

/// Write `text` to `.tmp/run-pin-file-{test}-{pid}/f.yml` (fresh dir).
fn scratch(test: &str, text: &str) -> PathBuf {
    let dir = PathBuf::from(format!(".tmp/run-pin-file-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("f.yml");
    std::fs::write(&p, text).unwrap();
    p
}

fn cleanup(p: &Path) {
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

const TEXT: &str = "image: abcdefgh\nnext: line\n";

#[tokio::test]
async fn rows_sorted_one_based_with_plugin_and_kind() {
    let p = scratch("rows", TEXT);
    let fake = Fake::new(vec![
        finding(3, Status::Ok, vec![]),
        finding(0, Status::Ok, vec![]),
    ]);
    let rows = pin_file(&fake, &ctx(), &p, &on(), &opts(4)).await.unwrap();
    assert_eq!(rows.iter().map(|r| r.line).collect::<Vec<_>>(), [1, 4]);
    assert_eq!(rows[0].reference, "ref0");
    assert!(rows.iter().all(|r| r.plugin == "fake" && r.kind == "x"));
    assert!(rows.iter().all(|r| r.file == p.display().to_string()));
    cleanup(&p);
}

#[tokio::test]
async fn writes_only_when_edits() {
    let p = scratch("writes", TEXT);
    let stamp = UNIX_EPOCH + Duration::from_secs(1000);
    std::fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    let mtime = |p: &Path| -> SystemTime { std::fs::metadata(p).unwrap().modified().unwrap() };

    let none = Fake::new(vec![finding(0, Status::Ok, vec![])]);
    pin_file(&none, &ctx(), &p, &on(), &opts(4)).await.unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), TEXT);
    assert_eq!(mtime(&p), stamp);

    let edits = vec![Splice::replace(7, 15, "pinned")];
    let one = Fake::new(vec![finding(0, Status::Pinned, edits.clone())]);
    pin_file(&one, &ctx(), &p, &on(), &opts(4)).await.unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), apply(TEXT, &edits));
    cleanup(&p);
}

#[tokio::test]
async fn dry_run_and_check_never_write() {
    for (dry_run, check) in [(true, false), (false, true)] {
        let p = scratch(&format!("nowrite-{dry_run}"), TEXT);
        let fake = Fake::new(vec![finding(
            0,
            Status::Pinned,
            vec![Splice::replace(7, 15, "pinned")],
        )]);
        let o = RunOptions {
            dry_run,
            check,
            concurrency: 4,
        };
        let rows = pin_file(&fake, &ctx(), &p, &on(), &o).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            TEXT,
            "dry_run={dry_run} check={check}"
        );
        assert_eq!(rows[0].status, Status::Pinned);
        cleanup(&p);
    }
}

#[tokio::test]
async fn partial_write_keeps_other_edits() {
    let p = scratch("partial", TEXT);
    let mut bad = finding(1, Status::Unresolved, vec![]);
    bad.reason = Some("not found".into());
    let edits = vec![Splice::replace(7, 15, "pinned")];
    let fake = Fake::new(vec![finding(0, Status::Pinned, edits.clone()), bad]);
    let rows = pin_file(&fake, &ctx(), &p, &on(), &opts(4)).await.unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), apply(TEXT, &edits));
    assert_eq!(rows[1].status, Status::Unresolved);
    cleanup(&p);
}

#[tokio::test]
async fn overlapping_edits_mark_both_unresolved() {
    let p = scratch("overlap", TEXT);
    let fake = Fake::new(vec![
        finding(0, Status::Pinned, vec![Splice::replace(5, 10, "aaaaa")]),
        finding(0, Status::Pinned, vec![Splice::replace(8, 12, "bbbb")]),
    ]);
    let rows = pin_file(&fake, &ctx(), &p, &on(), &opts(4)).await.unwrap();
    for r in &rows {
        assert_eq!(r.status, Status::Unresolved);
        assert_eq!(r.reason.as_deref(), Some("internal: overlapping edits"));
        assert_eq!(r.new, None);
    }
    assert_eq!(std::fs::read_to_string(&p).unwrap(), TEXT);
    cleanup(&p);

    // An insertion at the end of a replacement is adjacent, not overlapping.
    let p = scratch("adjacent", TEXT);
    let edits = vec![Splice::replace(5, 10, "aaaaa"), Splice::insert(10, "  # c")];
    let fake = Fake::new(vec![
        finding(0, Status::Pinned, vec![edits[0].clone()]),
        finding(0, Status::Pinned, vec![edits[1].clone()]),
    ]);
    let rows = pin_file(&fake, &ctx(), &p, &on(), &opts(4)).await.unwrap();
    assert!(rows.iter().all(|r| r.status == Status::Pinned));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), apply(TEXT, &edits));
    cleanup(&p);
}

#[tokio::test]
async fn read_error_is_file_error_with_path() {
    let p = PathBuf::from(format!(
        ".tmp/run-pin-file-missing-{}/absent.yml",
        std::process::id()
    ));
    let fake = Fake::new(vec![]);
    let err = pin_file(&fake, &ctx(), &p, &on(), &opts(4))
        .await
        .unwrap_err();
    let path = p.display().to_string();
    assert!(
        matches!(&err, FileError::Read { path: ep, .. } if *ep == path),
        "{err:?}"
    );
    assert!(err.to_string().starts_with(&path), "{err}");
}

#[tokio::test]
async fn plan_bounds_concurrency() {
    let p = scratch("bound", TEXT);
    let fake = Fake::new((0..5).map(|_| finding(0, Status::Ok, vec![])).collect());
    pin_file(&fake, &ctx(), &p, &on(), &opts(2)).await.unwrap();
    assert_eq!(fake.max_in_flight.get(), 2);
    cleanup(&p);
}

#[tokio::test]
async fn plan_concurrency_zero_is_one() {
    let p = scratch("zero", TEXT);
    let fake = Fake::new((0..3).map(|_| finding(0, Status::Ok, vec![])).collect());
    pin_file(&fake, &ctx(), &p, &on(), &opts(0)).await.unwrap();
    assert_eq!(fake.max_in_flight.get(), 1);
    cleanup(&p);
}

#[tokio::test]
async fn scan_error_is_file_error_and_no_write() {
    let p = scratch("scanerr", TEXT);
    let mut fake = Fake::new(vec![finding(
        0,
        Status::Pinned,
        vec![Splice::replace(7, 15, "pinned")],
    )]);
    fake.scan_error = Some(ScanError {
        line: Some(3),
        msg: "bad yaml".into(),
    });
    let err = pin_file(&fake, &ctx(), &p, &on(), &opts(4))
        .await
        .unwrap_err();
    let path = p.display().to_string();
    assert_eq!(
        err,
        FileError::Parse {
            path: path.clone(),
            line: Some(3),
            msg: "bad yaml".into()
        }
    );
    assert_eq!(err.to_string(), format!("{path}:3: bad yaml"));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), TEXT);
    assert_eq!(fake.max_in_flight.get(), 0);
    cleanup(&p);
}

/// Review L1 (Y-D12): the combined trailing comment lists originals in byte
/// order of their replacements, whatever order the plugin found them in;
/// rows keep finding order.
#[tokio::test]
async fn combined_comment_in_byte_order() {
    let text = "k: [p, q]\n";
    let p = scratch("comment-order", text);
    let named = |name: &str, edits: Vec<Splice>| {
        let mut f = finding(0, Status::Pinned, edits);
        f.reference = name.into();
        f.old = name.into();
        f
    };
    let fake = Fake::new(vec![
        named(
            "q",
            vec![Splice::replace(7, 8, "Q"), Splice::insert(9, "  # q")],
        ),
        named(
            "p",
            vec![Splice::replace(4, 5, "P"), Splice::insert(9, "  # p")],
        ),
    ]);
    let rows = pin_file(&fake, &ctx(), &p, &on(), &opts(4)).await.unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "k: [P, Q]  # p, q\n");
    assert_eq!(rows[0].reference, "q");
    assert_eq!(rows[1].reference, "p");
    cleanup(&p);
}

/// Review L7: an insertion strictly inside another finding's replacement is
/// an overlap (either finding order); both become `unresolved`, nothing is
/// written.
#[tokio::test]
async fn insertion_inside_other_replacement_marks_both_unresolved() {
    let text = "abcdefghij\n";
    let replace = Splice::replace(2, 6, "X");
    let insert = Splice::insert(4, "  # y");
    for (n, order) in [[&replace, &insert], [&insert, &replace]]
        .into_iter()
        .enumerate()
    {
        let p = scratch(&format!("insert-inside-{n}"), text);
        let fake = Fake::new(
            order
                .into_iter()
                .map(|s| finding(0, Status::Pinned, vec![s.clone()]))
                .collect(),
        );
        let rows = pin_file(&fake, &ctx(), &p, &on(), &opts(4)).await.unwrap();
        for r in &rows {
            assert_eq!(r.status, Status::Unresolved, "order {n}");
            assert_eq!(r.reason.as_deref(), Some("internal: overlapping edits"));
        }
        assert_eq!(std::fs::read_to_string(&p).unwrap(), text);
        cleanup(&p);
    }
}

/// Review L7: findings that overlap each other are marked, an independent
/// finding next to them is still written.
#[tokio::test]
async fn independent_finding_written_next_to_overlap() {
    let text = "abcdefghij\n";
    let p = scratch("independent", text);
    let fake = Fake::new(vec![
        finding(0, Status::Pinned, vec![Splice::replace(0, 3, "A")]),
        finding(0, Status::Pinned, vec![Splice::replace(2, 5, "B")]),
        finding(0, Status::Pinned, vec![Splice::replace(7, 9, "C")]),
    ]);
    let rows = pin_file(&fake, &ctx(), &p, &on(), &opts(4)).await.unwrap();
    let statuses: Vec<_> = rows.iter().map(|r| r.status).collect();
    assert_eq!(
        statuses,
        [Status::Unresolved, Status::Unresolved, Status::Pinned]
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "abcdefgCj\n");
    cleanup(&p);
}
