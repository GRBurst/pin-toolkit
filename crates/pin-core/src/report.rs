//! Result records and report rendering (human + JSON).

use std::collections::BTreeMap;

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
    /// The finding's follow-up note; collected into [`Report::follow_ups`].
    #[serde(skip)]
    pub follow_up: Option<String>,
}

/// A per-file failure (read, parse, write): the file is not processed, the
/// run goes on (spec §6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorEntry {
    /// As displayed (relative to the cwd, or as passed).
    pub file: String,
    /// 1-based, when the failure has a line (parse errors).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub message: String,
}

/// One plugin's part of the report.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PluginReport {
    pub summary: Summary,
    pub errors: Vec<ErrorEntry>,
}

/// The report of one run (v1 model, spec §6.4). `schema_version`,
/// `plugins` and `follow_ups` are not serialised yet (T-D19).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    #[serde(skip)]
    pub schema_version: u32,
    pub results: Vec<Row>,
    pub summary: Summary,
    #[serde(skip)]
    pub plugins: BTreeMap<String, PluginReport>,
    #[serde(skip)]
    pub follow_ups: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub pinned: usize,
    pub ok: usize,
    pub updated: usize,
    pub skipped: usize,
    pub unresolved: usize,
}

impl Summary {
    fn count(&mut self, status: Status) {
        match status {
            Status::Pinned => self.pinned += 1,
            Status::Ok => self.ok += 1,
            Status::Updated => self.updated += 1,
            Status::Skipped => self.skipped += 1,
            Status::Unresolved => self.unresolved += 1,
        }
    }
}

impl Report {
    pub const SCHEMA_VERSION: u32 = 1;

    /// Build a report, filling the summaries from the results (plugin
    /// entries derived from the rows' plugin ids, no errors).
    pub fn new(results: Vec<Row>) -> Self {
        Self::assemble(results, &[], vec![])
    }

    /// Build a report: the total summary and one entry per plugin in `ran`
    /// (plugins with at least one routed file, an entry even without rows)
    /// or with a row or an error; per-plugin summaries partition the total.
    /// Follow-ups: the rows' `follow_up`, deduplicated, first seen first.
    pub fn assemble(
        results: Vec<Row>,
        ran: &[&'static str],
        errors: Vec<(&'static str, ErrorEntry)>,
    ) -> Self {
        let mut summary = Summary::default();
        let mut plugins: BTreeMap<String, PluginReport> = ran
            .iter()
            .map(|p| (p.to_string(), PluginReport::default()))
            .collect();
        let mut follow_ups: Vec<String> = Vec::new();
        for r in &results {
            summary.count(r.status);
            plugins
                .entry(r.plugin.to_string())
                .or_default()
                .summary
                .count(r.status);
            if let Some(f) = &r.follow_up
                && !follow_ups.contains(f)
            {
                follow_ups.push(f.clone());
            }
        }
        for (p, e) in errors {
            plugins.entry(p.to_string()).or_default().errors.push(e);
        }
        Self {
            schema_version: Self::SCHEMA_VERSION,
            results,
            summary,
            plugins,
            follow_ups,
        }
    }

    /// Exit code (spec §6.5, P-exit without fatal errors): 2 if any file
    /// error; else 1 if anything is unresolved, or with `check` if anything
    /// would be pinned or updated; else 0.
    pub fn exit_code(&self, check: bool) -> u8 {
        if self.plugins.values().any(|p| !p.errors.is_empty()) {
            2
        } else if self.summary.unresolved > 0
            || (check && self.summary.pinned + self.summary.updated > 0)
        {
            1
        } else {
            0
        }
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
                follow_up: None,
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
                follow_up: None,
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
                follow_up: None,
            },
        ]);
        let expected = "ci/.gitlab-ci.yml:2  pinned  alpine:3.20 -> alpine:3.20@sha256:abc  # alpine:3.20\n\
                    ci/.gitlab-ci.yml:4  updated  alpine:3.20 -> alpine:3.24@sha256:def  # alpine:3.20\n\
                    ci/.gitlab-ci.yml:5  skipped (registry not enabled: ghcr.io)  ghcr.io/o/i:v1 -> -  # ghcr.io/o/i:v1\n\
                    pinned: 1, ok: 0, updated: 1, skipped: 1, unresolved: 0\n";
        assert_eq!(r.render_human(), expected);
    }

    #[test]
    fn status_serde_and_display() {
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
        }
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
            follow_up: None,
        }]);
        let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(v["summary"]["unresolved"], 1);
        assert_eq!(v["results"][0]["reason"], "boom");
    }

    const ALL: [Status; 5] = [
        Status::Pinned,
        Status::Ok,
        Status::Updated,
        Status::Skipped,
        Status::Unresolved,
    ];

    fn row_of(plugin: &'static str, status: Status, follow_up: Option<&str>) -> Row {
        Row {
            plugin,
            kind: "k",
            file: "f".into(),
            line: 1,
            reference: "r".into(),
            old: "r".into(),
            new: None,
            status,
            reason: None,
            follow_up: follow_up.map(String::from),
        }
    }

    fn entry(file: &str) -> ErrorEntry {
        ErrorEntry {
            file: file.into(),
            line: None,
            message: "m".into(),
        }
    }

    /// P-exit without "fatal": 2 iff a file error; else 1 iff unresolved,
    /// or `--check` and something would be pinned/updated; else 0.
    #[test]
    fn exit_code_rules() {
        fn reference(statuses: &[Status], check: bool, errors: bool) -> u8 {
            let has = |s: Status| statuses.contains(&s);
            if errors {
                2
            } else if has(Status::Unresolved)
                || (check && (has(Status::Pinned) || has(Status::Updated)))
            {
                1
            } else {
                0
            }
        }
        for mask in 0u32..(1 << ALL.len()) {
            let statuses: Vec<Status> = (0..ALL.len())
                .filter(|i| mask & (1 << i) != 0)
                .map(|i| ALL[i])
                .collect();
            let rows: Vec<Row> = statuses.iter().map(|s| row_of("p", *s, None)).collect();
            for check in [false, true] {
                for errors in [false, true] {
                    let errs = if errors {
                        vec![("p", entry("bad"))]
                    } else {
                        vec![]
                    };
                    let r = Report::assemble(rows.clone(), &["p"], errs);
                    assert_eq!(
                        r.exit_code(check),
                        reference(&statuses, check, errors),
                        "{statuses:?} check={check} errors={errors}"
                    );
                }
            }
        }
    }

    /// P-sum: per-plugin summaries partition the total; a plugin that ran
    /// without rows still has an entry.
    #[test]
    fn plugin_summaries_partition_total() {
        let rows = vec![
            row_of("a", Status::Pinned, None),
            row_of("b", Status::Skipped, None),
            row_of("a", Status::Unresolved, None),
            row_of("b", Status::Ok, None),
            row_of("a", Status::Updated, None),
        ];
        let r = Report::assemble(rows, &["a", "b", "c"], vec![("b", entry("x"))]);
        assert_eq!(r.schema_version, Report::SCHEMA_VERSION);
        assert_eq!(
            r.plugins.keys().map(String::as_str).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(r.plugins["c"], PluginReport::default());
        assert_eq!(r.plugins["b"].errors, [entry("x")]);
        let mut sum = Summary::default();
        for p in r.plugins.values() {
            sum.pinned += p.summary.pinned;
            sum.ok += p.summary.ok;
            sum.updated += p.summary.updated;
            sum.skipped += p.summary.skipped;
            sum.unresolved += p.summary.unresolved;
        }
        assert_eq!(sum, r.summary);
        assert_eq!(
            r.plugins["a"].summary,
            Summary {
                pinned: 1,
                ok: 0,
                updated: 1,
                skipped: 0,
                unresolved: 1
            }
        );
        // `new` derives the plugin entries from the rows.
        let n = Report::new(vec![row_of("a", Status::Ok, None)]);
        assert_eq!(n.plugins.keys().collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn follow_ups_dedup_first_seen() {
        let rows = vec![
            row_of("a", Status::Pinned, Some("x")),
            row_of("a", Status::Ok, None),
            row_of("a", Status::Pinned, Some("y")),
            row_of("a", Status::Pinned, Some("x")),
        ];
        let r = Report::assemble(rows, &["a"], vec![]);
        assert_eq!(r.follow_ups, ["x", "y"]);
    }
}
