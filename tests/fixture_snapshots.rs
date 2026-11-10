//! M5a: snapshot baseline. For every fixture and mode, three insta
//! snapshots: the human report, the JSON report and the edited file.
//!
//! Registries and GitLab are fakes (`common`): digests/SHAs derive from the
//! request, so a new fixture needs no new mock. Accept with
//! `INSTA_UPDATE=always`, review with `git diff`.
mod common;

use common::*;

#[tokio::test]
async fn fixture_snapshots() {
    let fakes = start_fakes().await;
    for fx in fixtures() {
        for m in modes(&fx.name) {
            let p = scratch_copy(&fx, m, "snap");
            let r = normalized(pin_once(&fakes, &p, m).await, &fx.name);
            let id = format!("{}__{}", fx.stem, m.tag());
            insta::assert_snapshot!(format!("{id}__human"), redact(&r.render_human()));
            insta::assert_snapshot!(format!("{id}__json"), redact(&r.to_json()));
            let edited = std::fs::read_to_string(&p).unwrap();
            insta::assert_snapshot!(format!("{id}__file"), visible(&edited));
            std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
        }
    }
}

#[test]
fn redact_masks_mock_ports() {
    assert_eq!(
        redact("error sending request for url (http://127.0.0.1:39373/v2/x)"),
        "error sending request for url (http://127.0.0.1:[PORT]/v2/x)"
    );
    assert_eq!(redact("no port here"), "no port here");
    assert_eq!(redact("http://127.0.0.1:/x"), "http://127.0.0.1:/x");
}

#[test]
fn visible_marks_cr_and_missing_newline() {
    assert_eq!(visible("a\r\nb"), "a␍\nb\n[no newline at end of file]");
    assert_eq!(visible("a\n"), "a\n");
}
