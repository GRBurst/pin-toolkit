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
/// (base `https://{host}/api/v4`); `GITLAB_TOKEN` takes precedence over
/// `CI_JOB_TOKEN` (M3 Task 3).
pub fn gitlab_from_env(host: &str) -> GitLabClient {
    GitLabClient::from_env(format!("https://{host}/api/v4"))
}

/// Pin every target in each file (files in order; resolutions within one
/// file run concurrently, bounded by `opts.concurrency`). The clients are
/// cloned into one [`Ctx`] (shared caches); the first file error aborts.
pub async fn pin_files(
    paths: &[PathBuf],
    opts: &PinOptions,
    reg: &RegistryClient,
    gitlab: &GitLabClient,
) -> Result<Report, String> {
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
        let file_rows = pin_file(&plugin, &ctx, path, &on, &run)
            .await
            .map_err(|e| e.to_string())?;
        rows.extend(file_rows);
    }
    Ok(Report::new(rows))
}

/// Exit code for a report: 1 when anything was unresolved, else 0.
pub fn exit_code(report: &Report) -> i32 {
    if report.summary.unresolved > 0 { 1 } else { 0 }
}
