//! Result records and report rendering (human + JSON).

use std::collections::{BTreeMap, BTreeSet};

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
    /// Plugins that had at least one routed file (review-2 L7): the
    /// human per-plugin summary lines appear when more than one ran. An
    /// entry in `plugins` holding only errors recorded outside the run
    /// (routing conflicts, walk errors) is not in here. Never serialised.
    #[serde(skip)]
    pub ran: BTreeSet<String>,
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
    /// [`Report::ran`] = `ran` plus the plugins of the rows.
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
        let mut ran_set: BTreeSet<String> = ran.iter().map(|p| p.to_string()).collect();
        let mut follow_ups: Vec<String> = Vec::new();
        for r in &results {
            ran_set.insert(r.plugin.to_string());
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
            ran: ran_set,
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

    /// Human report (T-D21, spec §6.4): one line per result, the total
    /// summary line, then one `{plugin}: …` summary line per plugin when
    /// more than one plugin ran ([`Report::ran`]), then the follow-ups:
    /// `{file}:{line}  {plugin}:{kind}  {status}{ (reason)}  {old} -> {new or "-"}  # {reference}`
    /// `pinned: {n}, ok: {n}, updated: {n}, skipped: {n}, unresolved: {n}`
    /// `{plugin}: pinned: {n}, …`
    /// `follow-up: {text}`
    pub fn render_human(&self) -> String {
        let mut out = String::new();
        for r in &self.results {
            let status = match &r.reason {
                Some(reason) => format!("{} ({})", r.status, reason),
                None => r.status.to_string(),
            };
            let new = r.new.as_deref().unwrap_or("-");
            out.push_str(&format!(
                "{}:{}  {}:{}  {}  {} -> {}  # {}\n",
                r.file, r.line, r.plugin, r.kind, status, r.old, new, r.reference
            ));
        }
        out.push_str(&summary_line(&self.summary));
        if self.ran.len() > 1 {
            for id in &self.ran {
                let s = self.plugins.get(id).map(|p| &p.summary);
                out.push_str(&format!(
                    "{id}: {}",
                    summary_line(s.unwrap_or(&Summary::default()))
                ));
            }
        }
        for f in &self.follow_ups {
            out.push_str(&format!("follow-up: {f}\n"));
        }
        out
    }

    /// `serde_json::to_string_pretty` of the report.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("Report serializes to JSON")
    }
}

/// `pinned: {n}, ok: {n}, updated: {n}, skipped: {n}, unresolved: {n}\n`
fn summary_line(s: &Summary) -> String {
    format!(
        "pinned: {}, ok: {}, updated: {}, skipped: {}, unresolved: {}\n",
        s.pinned, s.ok, s.updated, s.skipped, s.unresolved
    )
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
        let expected = "ci/.gitlab-ci.yml:2  gitlab-ci:image  pinned  alpine:3.20 -> alpine:3.20@sha256:abc  # alpine:3.20\n\
                    ci/.gitlab-ci.yml:4  gitlab-ci:image  updated  alpine:3.20 -> alpine:3.24@sha256:def  # alpine:3.20\n\
                    ci/.gitlab-ci.yml:5  gitlab-ci:image  skipped (registry not enabled: ghcr.io)  ghcr.io/o/i:v1 -> -  # ghcr.io/o/i:v1\n\
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

    /// T-D21 (§6.4, PR-L1): rows, the total line, one line per plugin when
    /// more than one plugin ran, then the follow-ups. "Ran" = had a routed
    /// file (review-2 L7): an entry holding only routing/walk errors does
    /// not count.
    #[test]
    fn human_render_multi_plugin() {
        let rows = vec![
            row_of("a", Status::Pinned, Some("x")),
            row_of("b", Status::Skipped, None),
            row_of("a", Status::Ok, Some("x")),
        ];
        let r = Report::assemble(rows.clone(), &["a", "b"], vec![]);
        assert_eq!(
            r.render_human(),
            "f:1  a:k  pinned  r -> -  # r\n\
             f:1  b:k  skipped  r -> -  # r\n\
             f:1  a:k  ok  r -> -  # r\n\
             pinned: 1, ok: 1, updated: 0, skipped: 1, unresolved: 0\n\
             a: pinned: 1, ok: 1, updated: 0, skipped: 0, unresolved: 0\n\
             b: pinned: 0, ok: 0, updated: 0, skipped: 1, unresolved: 0\n\
             follow-up: x\n"
        );
        // A plugin that ran over files without findings still counts.
        let quiet = Report::assemble(vec![row_of("a", Status::Ok, None)], &["a", "b"], vec![]);
        assert!(
            quiet
                .render_human()
                .ends_with("b: pinned: 0, ok: 0, updated: 0, skipped: 0, unresolved: 0\n"),
            "{}",
            quiet.render_human()
        );
        // Single plugin: rows and the total line only.
        let one = Report::assemble(vec![row_of("a", Status::Ok, None)], &["a"], vec![]);
        assert_eq!(
            one.render_human(),
            "f:1  a:k  ok  r -> -  # r\n\
             pinned: 0, ok: 1, updated: 0, skipped: 0, unresolved: 0\n"
        );
        // An entry with only errors recorded after the run (conflicts, walk
        // errors) is not a plugin that ran.
        let mut conflict = one.clone();
        conflict
            .plugins
            .entry("b".into())
            .or_default()
            .errors
            .push(entry("x"));
        assert_eq!(conflict.render_human(), one.render_human());
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
