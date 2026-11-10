//! M5a: properties every fixture must satisfy, derived from the code under
//! test (new fixtures are picked up by the glob in `common::fixtures`):
//! - byte-exact: an edit changes only reference bytes (plus one inserted
//!   trailing comment per line);
//! - idempotent: a second run edits nothing;
//! - one comment per line (R1, C11): that inserted comment has one `#`.
mod common;

use common::*;
use pin_core::report::{Report, Row, Status};

/// A pinned row on `line` (1-based) turning `old` into `new`.
fn row(line: usize, old: &str, new: &str) -> Row {
    Row {
        plugin: "gitlab-ci",
        kind: "image",
        file: "t.yml".into(),
        line,
        reference: old.into(),
        old: old.into(),
        new: Some(new.into()),
        status: Status::Pinned,
        reason: None,
    }
}

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[tokio::test]
async fn byte_exact_over_all_fixtures() {
    let fakes = start_fakes().await;
    let mut failures = Vec::new();
    for fx in fixtures() {
        for m in modes(&fx.name) {
            let p = scratch_copy(&fx, m, "bytes");
            let r1 = pin_once(&fakes, &p, m).await;
            if let Err(msg) = check_byte_exact(&fx.text, &read(&p), &r1) {
                failures.push(format!("{}/{}: {msg}", fx.name, m.tag()));
            }
            std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn idempotent_over_all_fixtures() {
    let fakes = start_fakes().await;
    let mut failures = Vec::new();
    for fx in fixtures() {
        for m in modes(&fx.name) {
            let p = scratch_copy(&fx, m, "idem");
            let r1 = pin_once(&fakes, &p, m).await;
            let e1 = read(&p);
            let r2 = pin_once(&fakes, &p, m).await;
            let e2 = read(&p);
            if let Err(msg) = check_idempotent(&r1, &e1, &r2, &e2) {
                failures.push(format!("{}/{}: {msg}", fx.name, m.tag()));
            }
            std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// R1 (C11): several refs pinned on one line share one trailing comment.
#[tokio::test]
async fn one_comment_per_line_over_all_fixtures() {
    let fakes = start_fakes().await;
    let mut failures = Vec::new();
    for fx in fixtures() {
        for m in modes(&fx.name) {
            let p = scratch_copy(&fx, m, "onecomment");
            let r1 = pin_once(&fakes, &p, m).await;
            match check_byte_exact(&fx.text, &read(&p), &r1) {
                Err(msg) => failures.push(format!("{}/{}: {msg}", fx.name, m.tag())),
                Ok(lines) => {
                    for (i, ins) in lines.iter().enumerate() {
                        if let Some(ins) = ins.as_ref().filter(|s| s.text.matches('#').count() != 1)
                        {
                            failures.push(format!(
                                "{}/{} line {}: inserted {:?}",
                                fx.name,
                                m.tag(),
                                i + 1,
                                ins.text
                            ));
                        }
                    }
                }
            }
            std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn checker_rejects_flipped_byte() {
    let o = "a: x\nb: y\n";
    let rows = Report::new(vec![row(1, "x", "x@sha256:1")]);

    let flipped = "a: x@sha256:1\nb: Y\n";
    assert!(check_byte_exact(o, flipped, &rows).is_err());

    let clean = "a: x@sha256:1\nb: y\n";
    assert_eq!(check_byte_exact(o, clean, &rows).unwrap(), vec![None, None]);

    let commented = "a: x@sha256:1  # c\nb: y\n";
    assert_eq!(
        check_byte_exact(o, commented, &rows).unwrap(),
        vec![
            Some(Insertion {
                at: 4,
                text: "  # c".into()
            }),
            None
        ]
    );

    let absent = Report::new(vec![row(1, "x", "x@sha256:2")]);
    assert!(check_byte_exact(o, clean, &absent).is_err());
}
