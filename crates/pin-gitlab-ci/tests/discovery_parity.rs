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
use pin_gitlab_ci::discover::{PinTarget, discover};

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
