//! Result records and report rendering (human + JSON).

use serde::Serialize;

/// Outcome of one reference. Serialises lowercase (`"pinned"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pinned,
    Ok,
    Updated,
    Skipped,
    Unresolved,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pinned => "pinned",
            Status::Ok => "ok",
            Status::Updated => "updated",
            Status::Skipped => "skipped",
            Status::Unresolved => "unresolved",
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<&str> for Status {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<str> for Status {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

/// One report row: one discovered reference and what happened to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    /// Plugin that found the reference. Not serialised until M5d.
    #[serde(skip)]
    pub plugin: &'static str,
    /// Reference kind within the plugin. Not serialised until M5d.
    #[serde(skip)]
    pub kind: &'static str,
    /// As passed on the CLI.
    pub file: String,
    /// 1-based.
    pub line: usize,
    pub reference: String,
    pub old: String,
    pub new: Option<String>,
    pub status: Status,
    /// Present for skipped/unresolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub results: Vec<Row>,
    pub summary: Summary,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub pinned: usize,
    pub ok: usize,
    pub updated: usize,
    pub skipped: usize,
    pub unresolved: usize,
}

impl Report {
    /// Build a report, filling the summary from the results.
    pub fn new(results: Vec<Row>) -> Self {
        let summary = Summary {
            pinned: results
                .iter()
                .filter(|r| r.status == Status::Pinned)
                .count(),
            ok: results.iter().filter(|r| r.status == Status::Ok).count(),
            updated: results
                .iter()
                .filter(|r| r.status == Status::Updated)
                .count(),
            skipped: results
                .iter()
                .filter(|r| r.status == Status::Skipped)
                .count(),
            unresolved: results
                .iter()
                .filter(|r| r.status == Status::Unresolved)
                .count(),
        };
        Self { results, summary }
    }

    /// One line per result, then the summary line:
    /// `{file}:{line}  {status}{(reason)}  {old} -> {new or "-"}  # {reference}`
    /// `pinned: {n}, ok: {n}, updated: {n}, skipped: {n}, unresolved: {n}`
    pub fn render_human(&self) -> String {
        let mut out = String::new();
        for r in &self.results {
            let status = match &r.reason {
                Some(reason) => format!("{} ({})", r.status, reason),
                None => r.status.to_string(),
            };
            let new = r.new.as_deref().unwrap_or("-");
            out.push_str(&format!(
                "{}:{}  {}  {} -> {}  # {}\n",
                r.file, r.line, status, r.old, new, r.reference
            ));
        }
        out.push_str(&format!(
            "pinned: {}, ok: {}, updated: {}, skipped: {}, unresolved: {}\n",
            self.summary.pinned,
            self.summary.ok,
            self.summary.updated,
            self.summary.skipped,
            self.summary.unresolved
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
            Row {
                plugin: "gitlab-ci",
                kind: "image",
                file: "ci/.gitlab-ci.yml".into(),
                line: 2,
                reference: "alpine:3.20".into(),
                old: "alpine:3.20".into(),
                new: Some("alpine:3.20@sha256:abc".into()),
                status: Status::Pinned,
                reason: None,
            },
            Row {
                plugin: "gitlab-ci",
                kind: "image",
                file: "ci/.gitlab-ci.yml".into(),
                line: 4,
                reference: "alpine:3.20".into(),
                old: "alpine:3.20".into(),
                new: Some("alpine:3.24@sha256:def".into()),
                status: Status::Updated,
                reason: None,
            },
            Row {
                plugin: "gitlab-ci",
                kind: "image",
                file: "ci/.gitlab-ci.yml".into(),
                line: 5,
                reference: "ghcr.io/o/i:v1".into(),
                old: "ghcr.io/o/i:v1".into(),
                new: None,
                status: Status::Skipped,
                reason: Some("registry not enabled: ghcr.io".into()),
            },
        ]);
        let expected = "ci/.gitlab-ci.yml:2  pinned  alpine:3.20 -> alpine:3.20@sha256:abc  # alpine:3.20\n\
                    ci/.gitlab-ci.yml:4  updated  alpine:3.20 -> alpine:3.24@sha256:def  # alpine:3.20\n\
                    ci/.gitlab-ci.yml:5  skipped (registry not enabled: ghcr.io)  ghcr.io/o/i:v1 -> -  # ghcr.io/o/i:v1\n\
                    pinned: 1, ok: 0, updated: 1, skipped: 1, unresolved: 0\n";
        assert_eq!(r.render_human(), expected);
    }

    #[test]
    fn status_serde_display_and_str_eq() {
        let all = [
            (Status::Pinned, "pinned"),
            (Status::Ok, "ok"),
            (Status::Updated, "updated"),
            (Status::Skipped, "skipped"),
            (Status::Unresolved, "unresolved"),
        ];
        for (status, name) in all {
            assert_eq!(
                serde_json::to_string(&status).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(status.to_string(), name);
            assert_eq!(status.as_str(), name);
            assert!(status == name);
            assert!(status == *name);
        }
        assert!(Status::Ok == "ok");
        assert!(Status::Ok != "skipped");
        assert_eq!(Status::Updated.as_str(), "updated");
    }

    #[test]
    fn json_render_roundtrip() {
        let r = Report::new(vec![Row {
            plugin: "gitlab-ci",
            kind: "image",
            file: "f".into(),
            line: 1,
            reference: "x".into(),
            old: "x".into(),
            new: None,
            status: Status::Unresolved,
            reason: Some("boom".into()),
        }]);
        let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(v["summary"]["unresolved"], 1);
        assert_eq!(v["results"][0]["reason"], "boom");
    }
}
