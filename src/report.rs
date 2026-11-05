//! Result records and report rendering (human + JSON).

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RefResult {
    /// As passed on the CLI.
    pub file: String,
    /// 1-based.
    pub line: usize,
    pub reference: String,
    pub old: String,
    pub new: Option<String>,
    /// "pinned" | "ok" | "skipped" | "unresolved"
    pub status: String,
    /// Present for skipped/unresolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub results: Vec<RefResult>,
    pub summary: Summary,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub pinned: usize,
    pub ok: usize,
    pub skipped: usize,
    pub unresolved: usize,
}

impl Report {
    /// Build a report, filling the summary from the results.
    pub fn new(results: Vec<RefResult>) -> Self {
        let summary = Summary {
            pinned: results.iter().filter(|r| r.status == "pinned").count(),
            ok: results.iter().filter(|r| r.status == "ok").count(),
            skipped: results.iter().filter(|r| r.status == "skipped").count(),
            unresolved: results.iter().filter(|r| r.status == "unresolved").count(),
        };
        Self { results, summary }
    }

    /// One line per result, then the summary line:
    /// `{file}:{line}  {status}{(reason)}  {old} -> {new or "-"}  # {reference}`
    /// `pinned: {n}, ok: {n}, skipped: {n}, unresolved: {n}`
    pub fn render_human(&self) -> String {
        let mut out = String::new();
        for r in &self.results {
            let status = match &r.reason {
                Some(reason) => format!("{} ({})", r.status, reason),
                None => r.status.clone(),
            };
            let new = r.new.as_deref().unwrap_or("-");
            out.push_str(&format!(
                "{}:{}  {}  {} -> {}  # {}\n",
                r.file, r.line, status, r.old, new, r.reference
            ));
        }
        out.push_str(&format!(
            "pinned: {}, ok: {}, skipped: {}, unresolved: {}\n",
            self.summary.pinned, self.summary.ok, self.summary.skipped, self.summary.unresolved
        ));
        out
    }

    /// `serde_json::to_string_pretty` of the report.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("Report serializes to JSON")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_render_exact() {
        let r = Report::new(vec![
            RefResult {
                file: "ci/.gitlab-ci.yml".into(),
                line: 2,
                reference: "alpine:3.20".into(),
                old: "alpine:3.20".into(),
                new: Some("alpine:3.20@sha256:abc".into()),
                status: "pinned".into(),
                reason: None,
            },
            RefResult {
                file: "ci/.gitlab-ci.yml".into(),
                line: 5,
                reference: "ghcr.io/o/i:v1".into(),
                old: "ghcr.io/o/i:v1".into(),
                new: None,
                status: "skipped".into(),
                reason: Some("registry not enabled: ghcr.io".into()),
            },
        ]);
        let expected = "ci/.gitlab-ci.yml:2  pinned  alpine:3.20 -> alpine:3.20@sha256:abc  # alpine:3.20\n\
                    ci/.gitlab-ci.yml:5  skipped (registry not enabled: ghcr.io)  ghcr.io/o/i:v1 -> -  # ghcr.io/o/i:v1\n\
                    pinned: 1, ok: 0, skipped: 1, unresolved: 0\n";
        assert_eq!(r.render_human(), expected);
    }

    #[test]
    fn json_render_roundtrip() {
        let r = Report::new(vec![RefResult {
            file: "f".into(),
            line: 1,
            reference: "x".into(),
            old: "x".into(),
            new: None,
            status: "unresolved".into(),
            reason: Some("boom".into()),
        }]);
        let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(v["summary"]["unresolved"], 1);
        assert_eq!(v["results"][0]["reason"], "boom");
    }
}
