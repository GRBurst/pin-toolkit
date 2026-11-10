//! Test and `pin-ci` binary helper until M5d (spec §9): runs [`GitlabCi`]
//! through [`pin_core::run::pin_file`] with preset clients. Not API.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pin_core::gitlab::GitLabClient;
use pin_core::oci::RegistryClient;
use pin_core::oci::registry::Creds;
use pin_core::plugin::KindSet;
use pin_core::report::Report;
use pin_core::run::{Ctx, ResolverEnv, RunOptions, UpdatePolicy, now_unix, pin_file};

use crate::{GitlabCi, KINDS};

/// Options for a pin run.
#[derive(Debug, Clone)]
pub struct PinOptions {
    /// Resolve and report without writing files.
    pub dry_run: bool,
    /// Report as JSON (rendering choice, handled by the CLI).
    pub json: bool,
    /// Max concurrent digest lookups per file.
    pub concurrency: usize,
    /// Bump unpinned, versioned image refs (any registry host) to the
    /// latest compatible stable tag and pin the bumped tag's digest
    /// (status `updated`).
    pub update: bool,
    /// With `update`, allow bumping to a higher major (same suffix and
    /// release length still required). Meaningful only when `update` is set.
    pub update_major: bool,
    /// GitLab instance host for component/include resolution (default
    /// `gitlab.com`; set via `--gitlab-host`).
    pub gitlab_host: String,
    /// With `update`, skip bump candidates younger than N days (Docker Hub
    /// refs only; 0 = off; recommended 14). M4b-1 release-age guard.
    pub min_age_days: u64,
}

/// Every field at its off/default value (the CLI defaults).
impl Default for PinOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            json: false,
            concurrency: 8,
            update: false,
            update_major: false,
            gitlab_host: "gitlab.com".to_string(),
            min_age_days: 0,
        }
    }
}

/// Build the registry client from the environment: per-host credentials via
/// [`Creds::load_from`] (`DOCKERHUB_USER`/`DOCKERHUB_TOKEN` for `docker.io`,
/// `GITHUB_TOKEN` for `ghcr.io`, plus `~/.docker/config.json`); base URLs
/// default to the Docker Hub base for `docker.io` and `https://{host}` for
/// every other host (M4a Task 2).
pub fn client_from_env() -> RegistryClient {
    let creds = Creds::load_from(std::env::var("HOME").ok().as_deref().map(Path::new));
    RegistryClient::new(creds, HashMap::new())
}

/// Build the GitLab client for instance `host` from the environment
/// (base `https://{host}/api/v4`) with the credential bound to it: with
/// `host` as the primary, `GITLAB_TOKEN`; else `CI_JOB_TOKEN` only when
/// `CI_SERVER_HOST` is `host` (D31, D37).
pub fn gitlab_from_env(host: &str) -> GitLabClient {
    let (token, job) = ResolverEnv::from_process(host)
        .gitlab
        .auth_for(host)
        .into_parts();
    GitLabClient::try_new(format!("https://{host}/api/v4"), token, job)
        .expect("reqwest client builds")
}

/// A file error that stopped the run (the first one aborts it until M5d):
/// the message, and the report of the files processed before it.
#[derive(Debug)]
pub struct PinFilesError {
    /// Rows of the files before the failing one (already written, unless
    /// `dry_run`).
    pub partial: Report,
    /// The failing file's error (`<path>:<line>: …`).
    pub error: String,
}

impl std::fmt::Display for PinFilesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error)
    }
}

impl std::error::Error for PinFilesError {}

/// Pin every target in each file (files in order; resolutions within one
/// file run concurrently, bounded by `opts.concurrency`). The clients are
/// cloned into one [`Ctx`] (shared caches); the first file error aborts, and
/// carries the rows of the files before it (review L2).
pub async fn pin_files(
    paths: &[PathBuf],
    opts: &PinOptions,
    reg: &RegistryClient,
    gitlab: &GitLabClient,
) -> Result<Report, PinFilesError> {
    let update = UpdatePolicy {
        enabled: opts.update,
        allow_major: opts.update_major,
        min_age_days: u32::try_from(opts.min_age_days).unwrap_or(u32::MAX),
    };
    let ctx = Ctx::new(
        update,
        now_unix(),
        ResolverEnv::from_process(&opts.gitlab_host),
    )
    .with_oci(Ok(reg.clone()))
    .with_gitlab(&opts.gitlab_host, Ok(gitlab.clone()));
    let plugin = GitlabCi {
        gitlab_hosts: vec![opts.gitlab_host.clone()],
    };
    let on = KindSet::defaults(&KINDS);
    let run = RunOptions {
        dry_run: opts.dry_run,
        check: false,
        concurrency: opts.concurrency,
    };
    let mut rows = Vec::new();
    for path in paths {
        match pin_file(&plugin, &ctx, path, &on, &run).await {
            Ok(file_rows) => rows.extend(file_rows),
            Err(e) => {
                return Err(PinFilesError {
                    partial: Report::new(rows),
                    error: e.to_string(),
                });
            }
        }
    }
    Ok(Report::new(rows))
}

/// What `pin-ci` prints and exits with.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The report (rows, or rows of the files before a file error), when
    /// there is anything to show.
    pub stdout: Option<String>,
    /// `pin-ci: <error>` after a file error.
    pub stderr: Option<String>,
    pub code: i32,
}

/// The output and exit code of a run: the report as text or JSON and
/// [`exit_code`]; after a file error the rows of the files before it, then
/// the error, and exit 1.
pub fn outcome(result: Result<Report, PinFilesError>, json: bool) -> Outcome {
    let render = |r: &Report| if json { r.to_json() } else { r.render_human() };
    match result {
        Ok(report) => Outcome {
            stdout: Some(render(&report)),
            stderr: None,
            code: exit_code(&report),
        },
        Err(e) => Outcome {
            stdout: (!e.partial.results.is_empty()).then(|| render(&e.partial)),
            stderr: Some(format!("pin-ci: {}", e.error)),
            code: 1,
        },
    }
}

/// Exit code for a report: 1 when anything was unresolved, else 0.
pub fn exit_code(report: &Report) -> i32 {
    if report.summary.unresolved > 0 { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pin_core::report::{Row, Status};

    fn row() -> Row {
        Row {
            plugin: "gitlab-ci",
            kind: "image",
            file: "a.yml".into(),
            line: 2,
            reference: "x".into(),
            old: "x".into(),
            new: None,
            status: Status::Skipped,
            reason: Some("r".into()),
        }
    }

    /// Review L2: what `pin-ci` prints and exits with.
    #[test]
    fn outcome_prints_partial_rows_before_the_file_error() {
        let ok = outcome(Ok(Report::new(vec![row()])), false);
        assert_eq!(ok.code, 0);
        assert!(ok.stderr.is_none());
        assert!(ok.stdout.unwrap().starts_with("a.yml:2  skipped (r)"));

        let err = |rows: Vec<Row>| PinFilesError {
            partial: Report::new(rows),
            error: "b.yml:1: invalid YAML: boom".into(),
        };
        let out = outcome(Err(err(vec![row()])), false);
        assert_eq!(out.code, 1);
        assert_eq!(
            out.stderr.as_deref(),
            Some("pin-ci: b.yml:1: invalid YAML: boom")
        );
        assert!(out.stdout.unwrap().starts_with("a.yml:2  skipped (r)"));

        let out = outcome(Err(err(vec![row()])), true);
        let json: serde_json::Value = serde_json::from_str(&out.stdout.unwrap()).unwrap();
        assert_eq!(json["results"][0]["file"], "a.yml");

        // nothing processed before the error: nothing to print
        let out = outcome(Err(err(vec![])), false);
        assert_eq!((out.code, out.stdout), (1, None));
        assert!(out.stderr.is_some());
    }
}
