//! The global layer: `<config dir>/pin-toolkit/config.toml` (spec §7.1,
//! §7.2; M5e'). A `toml::Table` pre-pass gives the `version` messages,
//! then a typed parse with `deny_unknown_fields` rejects every other
//! unknown key (D17). Pure: the caller reads the file and names it.

use std::path::Path;

use serde::Deserialize;
use serde::de::IgnoredAny;

use super::{Fatal, Layer};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobalFile {
    #[serde(rename = "version")]
    _version: IgnoredAny,
    json: Option<bool>,
    concurrency: Option<usize>,
}

/// Parse the global config `text` read from `display` (shown in messages).
pub fn parse(display: &Path, text: &str) -> Result<Layer, Fatal> {
    let p = display.display();
    let fatal = |msg: String| Fatal(format!("{p}: {msg}"));
    let table: toml::Table =
        toml::from_str(text).map_err(|e| fatal(e.to_string().trim_end().to_string()))?;
    match table.get("version") {
        None => return Err(fatal("missing `version = 1`".into())),
        Some(toml::Value::Integer(1)) => {}
        Some(v) => return Err(fatal(format!("unsupported version {v} (expected 1)"))),
    }
    let file: GlobalFile =
        toml::from_str(text).map_err(|e| fatal(e.to_string().trim_end().to_string()))?;
    Ok(Layer {
        json: file.json,
        concurrency: file.concurrency,
        ..Layer::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "home/.config/pin-toolkit/config.toml";

    fn parse_text(text: &str) -> Result<Layer, Fatal> {
        parse(Path::new(PATH), text)
    }

    #[test]
    fn global_json_and_concurrency() {
        let layer = parse_text("version = 1\njson = true\nconcurrency = 4\n").unwrap();
        assert_eq!(
            layer,
            Layer {
                json: Some(true),
                concurrency: Some(4),
                ..Layer::default()
            }
        );
        assert_eq!(parse_text("version = 1\n"), Ok(Layer::default()));
    }

    #[test]
    fn global_rejects() {
        let rows = [
            ("json = true\n", "missing `version = 1`"),
            ("version = 2\n", "unsupported version 2 (expected 1)"),
            (
                "version = \"1\"\n",
                "unsupported version \"1\" (expected 1)",
            ),
            ("version = 1\njsno = true\n", "unknown field `jsno`"),
            (
                "version = 1\n[update]\nenabled = true\n",
                "unknown field `update`",
            ),
            ("version = 1\nconcurrency = -1\n", "expected usize"),
            ("version = 1\njson = \"yes\"\n", "invalid type"),
            ("version = [\n", "expected"),
        ];
        for (text, want) in rows {
            let got = parse_text(text).expect_err(text).0;
            assert!(got.starts_with(&format!("{PATH}: ")), "{text}: {got}");
            assert!(got.contains(want), "{text}: {got}");
        }
    }
}
