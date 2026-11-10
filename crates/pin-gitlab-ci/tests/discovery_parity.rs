//! M5c-y: discovery parity. The hand-rolled `discover()` is deleted at the
//! end of M5c-y (spec §9), so its output over every fixture is recorded in
//! `tests/parity/legacy-discover.txt` (written once, in Y1, by the old code,
//! then frozen). Later tasks compare the `core::yaml` discovery against that
//! record, modulo an exact exception table.
//!
//! Record format: line 1 is a header comment; then, per fixture (sorted),
//! `## {name} bytes={len} hash={fake_hex(text, 16)}` followed by one
//! [`render`]ed line per target.
mod common;

use common::*;
use pin_core::text::LineIndex;
use pin_gitlab_ci::discover::{PinTarget, discover, discover_yaml};

const RECORD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/parity/legacy-discover.txt"
);
const RECORD_HEADER: &str =
    "# legacy discover() output, recorded in M5c-y Y1 before its deletion; never regenerate";

fn kind(t: &PinTarget) -> &'static str {
    match t {
        PinTarget::Image { .. } => "image",
        PinTarget::Component { .. } => "component",
        PinTarget::IncludeRef { .. } => "include",
        PinTarget::Unsupported { kind, .. } => kind,
    }
}

/// One record line per target:
/// `{name}:{line+1}\t{kind}\t{start}..{end}\t{token:?}\t{detail}`.
fn render(name: &str, text: &str, ts: &[PinTarget]) -> Vec<String> {
    ts.iter()
        .map(|t| {
            let detail = match t {
                PinTarget::Image { value, .. } => format!("value={value:?}"),
                PinTarget::Component { value, parts, .. } => match parts {
                    Some(p) => format!(
                        "value={value:?} parts={}|{}|{}|{}",
                        p.host, p.project, p.name, p.version
                    ),
                    None => format!("value={value:?} parts=-"),
                },
                PinTarget::IncludeRef {
                    value,
                    project,
                    flow,
                    ..
                } => format!("value={value:?} project={project:?} flow={flow}"),
                PinTarget::Unsupported { value, reason, .. } => {
                    format!("value={value:?} reason={reason}")
                }
            };
            format!(
                "{name}:{}\t{}\t{}..{}\t{:?}\t{detail}",
                t.line() + 1,
                kind(t),
                t.start(),
                t.end(),
                &text[t.start()..t.end()],
            )
        })
        .collect()
}

/// The record text built from the current `discover()`.
fn legacy_record() -> String {
    let mut out = format!("{RECORD_HEADER}\n");
    for fx in fixtures() {
        out.push_str(&format!(
            "## {} bytes={} hash={}\n",
            fx.name,
            fx.text.len(),
            fake_hex(&fx.text, 16)
        ));
        for line in render(&fx.name, &fx.text, &discover(&fx.text)) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// Generator + guard: with `PIN_PARITY_RECORD=1` writes the record, otherwise
/// asserts the file still equals the old code's output. Removed with the old
/// code (Y11); the record file stays.
#[test]
fn legacy_record_matches_discover() {
    let expected = legacy_record();
    if std::env::var_os("PIN_PARITY_RECORD").is_some() {
        std::fs::create_dir_all(std::path::Path::new(RECORD).parent().unwrap()).unwrap();
        std::fs::write(RECORD, &expected).unwrap();
        return;
    }
    let recorded = std::fs::read_to_string(RECORD)
        .unwrap_or_else(|e| panic!("cannot read {RECORD}: {e} (PIN_PARITY_RECORD=1 writes it)"));
    assert_eq!(recorded, expected);
}

/// Kinds compared by [`yaml_discovery_matches_legacy_except_documented`];
/// `include` joins in Y9.
const PARITY_KINDS: &[&str] = &["image", "component"];

/// `(fixture, 1-based line, kind, raw token)`: matched on the record's
/// `name:line`, `kind` and `{token:?}` columns.
type Exc = (&'static str, usize, &'static str, &'static str);

/// Legacy lines that disappear.
const REMOVED: &[Exc] = &[
    ("ci-anchors.yml", 1, "image", "&img alpine:3.20"), // R7: anchor inside the value
    ("ci-anchors.yml", 6, "image", "*img"),             // R7: alias is not a target
    ("ci-anchors.yml", 9, "image", "&redis redis:7"),   // R7
    ("ci-anchors.yml", 10, "image", "*redis"),          // R7
    ("ci-tags.yml", 5, "image", "!reference [.base, image]"), // R7: !reference is not a target
    ("ci-tags.yml", 6, "image", "!reference [.base, services]"), // R7
    ("ci-tags.yml", 8, "image", "!!str node:20"),       // R7: tag inside the value
];

/// New lines that appear.
const ADDED: &[Exc] = &[
    ("ci-anchors.yml", 1, "image", "alpine:3.20"),  // R7
    ("ci-anchors.yml", 3, "image", "ubuntu:24.04"), // R8: child of `.base: &base`
    ("ci-anchors.yml", 9, "image", "redis:7"),      // R7
    ("ci-r2-crlf.yml", 6, "include", "dev"),        // R8: CRLF block include
    ("ci-r2-crlf.yml", 9, "image", "redis:7"),      // R8: CRLF block services
    ("ci-r2-crlf.yml", 10, "image", "\"node:20\""), // R8: CRLF job-nested image
    ("ci-tags.yml", 8, "image", "node:20"),         // R7
    ("ci-bom.yml", 2, "image", "alpine:3.20"),      // R9: BOM hid the first key's children
];

/// One `## name bytes=N hash=H` section of the record.
struct Recorded {
    name: String,
    bytes: usize,
    hash: String,
    lines: Vec<String>,
}

fn read_record() -> Vec<Recorded> {
    let text = std::fs::read_to_string(RECORD).unwrap();
    let mut out: Vec<Recorded> = Vec::new();
    for line in text.lines().skip(1) {
        if let Some(h) = line.strip_prefix("## ") {
            let mut parts = h.split(' ');
            let name = parts.next().unwrap().to_string();
            let bytes = parts.next().unwrap().strip_prefix("bytes=").unwrap();
            let hash = parts.next().unwrap().strip_prefix("hash=").unwrap();
            out.push(Recorded {
                name,
                bytes: bytes.parse().unwrap(),
                hash: hash.to_string(),
                lines: Vec::new(),
            });
        } else {
            out.last_mut().unwrap().lines.push(line.to_string());
        }
    }
    out
}

fn line_kind(line: &str) -> &str {
    line.split('\t').nth(1).unwrap()
}

fn matches_exc(line: &str, e: &Exc) -> bool {
    let f: Vec<&str> = line.split('\t').collect();
    f[0] == format!("{}:{}", e.0, e.1) && f[1] == e.2 && f[3] == format!("{:?}", e.3)
}

/// P-parity: per recorded fixture, `legacy ∖ REMOVED` equals
/// `render(discover_yaml) ∖ ADDED` as ordered lists, and every exception
/// matches exactly one line (a stale exception is a failure too).
#[test]
fn yaml_discovery_matches_legacy_except_documented() {
    let fixtures = fixtures();
    let mut failures = Vec::new();
    for rec in read_record() {
        let Some(fx) = fixtures.iter().find(|f| f.name == rec.name) else {
            failures.push(format!("record names a missing fixture {}", rec.name));
            continue;
        };
        if rec.bytes != fx.text.len() || rec.hash != fake_hex(&fx.text, 16) {
            failures.push(format!(
                "fixture {} changed after the legacy record",
                rec.name
            ));
            continue;
        }
        let new = match discover_yaml(&fx.text) {
            Ok(t) => render(&fx.name, &fx.text, &t),
            Err(e) => {
                failures.push(format!("{}: discover_yaml failed: {e:?}", fx.name));
                continue;
            }
        };
        let kept = |l: &&String| PARITY_KINDS.contains(&line_kind(l));
        let legacy: Vec<&String> = rec.lines.iter().filter(kept).collect();
        let new: Vec<&String> = new.iter().filter(kept).collect();
        let exc = |list: &'static [Exc]| -> Vec<&'static Exc> {
            list.iter()
                .filter(|e| e.0 == rec.name && PARITY_KINDS.contains(&e.2))
                .collect()
        };
        let (removed, added) = (exc(REMOVED), exc(ADDED));
        for (what, list, lines) in [("REMOVED", &removed, &legacy), ("ADDED", &added, &new)] {
            for e in list {
                let n = lines.iter().filter(|l| matches_exc(l, e)).count();
                if n != 1 {
                    failures.push(format!("{what} {e:?} matches {n} lines, expected 1"));
                }
            }
        }
        let rest = |lines: &[&String], list: &[&Exc]| -> Vec<String> {
            lines
                .iter()
                .filter(|l| !list.iter().any(|e| matches_exc(l, e)))
                .map(|l| l.to_string())
                .collect()
        };
        let (legacy_rest, new_rest) = (rest(&legacy, &removed), rest(&new, &added));
        for l in legacy_rest.iter().filter(|l| !new_rest.contains(l)) {
            failures.push(format!("unmatched legacy line: {l}"));
        }
        for l in new_rest.iter().filter(|l| !legacy_rest.contains(l)) {
            failures.push(format!("unmatched yaml line: {l}"));
        }
        if legacy_rest != new_rest && failures.is_empty() {
            failures.push(format!("{}: same lines, different order", rec.name));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// P-token: every spliceable target's span is its value as written (plain,
/// `'…'` or `"…"`), starts on the line it reports and never spans a line
/// break.
#[test]
fn spans_are_tokens_over_all_fixtures() {
    let mut failures = Vec::new();
    for fx in fixtures() {
        let targets = match discover_yaml(&fx.text) {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("{}: discover_yaml failed: {e:?}", fx.name));
                continue;
            }
        };
        let lines = LineIndex::new(&fx.text);
        for t in targets {
            if matches!(t, PinTarget::Unsupported { .. }) {
                continue;
            }
            let tok = &fx.text[t.start()..t.end()];
            let v = t.value();
            let at = format!("{}:{}", fx.name, t.line() + 1);
            if tok != v && tok != format!("'{v}'") && tok != format!("\"{v}\"") {
                failures.push(format!("{at}: token {tok:?} is not value {v:?}"));
            }
            if lines.line_of(t.start()) != t.line() {
                failures.push(format!("{at}: start {} is not on its line", t.start()));
            }
            if lines.line_end(t.line()).is_none_or(|end| t.end() > end) {
                failures.push(format!("{at}: token {tok:?} crosses the line end"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
