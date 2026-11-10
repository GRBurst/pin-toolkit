//! One file through one plugin (spec §6.2/§6.3): read, plan (scan → bounded
//! resolve), overlap check, write, rows. `Ctx` carries the run's policy.

#[cfg(any(feature = "oci", feature = "gitlab"))]
use std::cell::OnceCell;
#[cfg(feature = "oci")]
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::edit::{Splice, apply, coalesce_comments};
#[cfg(feature = "gitlab")]
use crate::gitlab::{GitLabClient, GitLabHosts};
#[cfg(feature = "oci")]
use crate::oci::ImageRef;
#[cfg(feature = "oci")]
use crate::oci::registry::{Creds, RegError, RegistryClient};
use crate::plugin::{DynPlugin, Finding, KindSet};
use crate::report::{Row, Status};

/// `--update` policy for one run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdatePolicy {
    pub enabled: bool,
    pub allow_major: bool,
    /// Release-age guard in days (0 = off).
    pub min_age_days: u32,
}

/// What resolvers are built from: read once per run.
#[derive(Debug, Clone, Default)]
pub struct ResolverEnv {
    /// Home directory (`~/.docker/config.json` credentials).
    pub home: Option<PathBuf>,
    /// The primary GitLab instance (`--gitlab-host`).
    pub gitlab_host: String,
}

impl ResolverEnv {
    /// `home` from `$HOME`; tokens are read by the client builders.
    pub fn from_process(gitlab_host: &str) -> Self {
        Self {
            home: std::env::var("HOME").ok().map(PathBuf::from),
            gitlab_host: gitlab_host.to_string(),
        }
    }
}

/// Per-run context: policy, clock and lazily built, feature-gated resolvers
/// (D36). Plugins resolve only through its entry points (D38).
pub struct Ctx {
    pub update: UpdatePolicy,
    pub now_unix: i64,
    env: ResolverEnv,
    #[cfg(feature = "oci")]
    oci: OnceCell<Result<RegistryClient, String>>,
    #[cfg(feature = "gitlab")]
    gitlab: OnceCell<Result<GitLabHosts, String>>,
}

impl Ctx {
    /// Builds nothing: resolvers are created on first use.
    pub fn new(update: UpdatePolicy, now_unix: i64, env: ResolverEnv) -> Self {
        Self {
            update,
            now_unix,
            env,
            #[cfg(feature = "oci")]
            oci: OnceCell::new(),
            #[cfg(feature = "gitlab")]
            gitlab: OnceCell::new(),
        }
    }

    pub fn env(&self) -> &ResolverEnv {
        &self.env
    }

    /// The resolvers built so far (attempted, including construction
    /// errors), in the order `oci`, `gitlab`.
    pub fn resolvers_built(&self) -> Vec<&'static str> {
        #[allow(unused_mut)] // no resolver feature: nothing is pushed
        let mut v = Vec::new();
        #[cfg(feature = "oci")]
        if self.oci.get().is_some() {
            v.push("oci");
        }
        #[cfg(feature = "gitlab")]
        if self.gitlab.get().is_some() {
            v.push("gitlab");
        }
        v
    }

    /// The registry client, built from `env` on first use; a construction
    /// error is cached.
    #[cfg(feature = "oci")]
    pub(crate) fn oci(&self) -> Result<&RegistryClient, &str> {
        self.oci
            .get_or_init(|| {
                Ok(RegistryClient::new(
                    Creds::load_from(self.env.home.as_deref()),
                    HashMap::new(),
                ))
            })
            .as_ref()
            .map_err(String::as_str)
    }

    /// Preset the registry client (or its construction error).
    #[cfg(feature = "oci")]
    pub fn with_oci(self, c: Result<RegistryClient, String>) -> Self {
        Self {
            oci: OnceCell::from(c),
            ..self
        }
    }

    /// The GitLab clients, built from `env` on first use; a construction
    /// error is cached.
    #[cfg(feature = "gitlab")]
    pub(crate) fn gitlab(&self) -> Result<&GitLabHosts, &str> {
        self.gitlab
            .get_or_init(|| {
                let host = &self.env.gitlab_host;
                Ok(GitLabHosts::single(
                    host,
                    GitLabClient::from_env(format!("https://{host}/api/v4")),
                ))
            })
            .as_ref()
            .map_err(String::as_str)
    }

    /// Preset the client of the primary GitLab host (or its construction
    /// error).
    #[cfg(feature = "gitlab")]
    pub fn with_gitlab(self, primary_host: &str, c: Result<GitLabClient, String>) -> Self {
        Self {
            gitlab: OnceCell::from(c.map(|c| GitLabHosts::single(primary_host, c))),
            ..self
        }
    }

    /// Manifest digest of `r`; a construction error is `RegError::Unresolved`.
    #[cfg(feature = "oci")]
    pub async fn oci_digest(&self, r: &ImageRef) -> Result<String, RegError> {
        self.oci()
            .map_err(|e| RegError::Unresolved(e.to_string()))?
            .digest(r)
            .await
    }

    /// Tag list of `r`'s repository.
    #[cfg(feature = "oci")]
    pub async fn oci_tags(&self, r: &ImageRef) -> Result<Vec<String>, RegError> {
        self.oci()
            .map_err(|e| RegError::Unresolved(e.to_string()))?
            .list_tags(r)
            .await
    }

    /// Tag push times (unix seconds) of `r`'s repository (Docker Hub API).
    #[cfg(feature = "oci")]
    pub async fn oci_tag_times(&self, r: &ImageRef) -> Result<BTreeMap<String, i64>, RegError> {
        self.oci()
            .map_err(|e| RegError::Unresolved(e.to_string()))?
            .tag_timestamps(r)
            .await
    }

    /// Resolve `git_ref` in `project` on GitLab `host` (`None` = the primary
    /// host) to a commit SHA. Errors are the client's texts, the cached
    /// construction error, or `gitlab: no client for {host}`.
    #[cfg(feature = "gitlab")]
    pub async fn gitlab_resolve(
        &self,
        host: Option<&str>,
        project: &str,
        git_ref: &str,
    ) -> Result<String, String> {
        let hosts = self.gitlab().map_err(str::to_string)?;
        let client = match host {
            None => hosts.primary(),
            Some(h) => hosts
                .get(h)
                .ok_or_else(|| format!("gitlab: no client for {h}"))?,
        };
        client
            .resolve(project, git_ref)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Current unix time in seconds from the system clock (M4b-1 release-age
/// guard); `0` if the clock predates the epoch (impossible in practice).
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// How a file is processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    pub dry_run: bool,
    pub check: bool,
    /// Max resolutions in flight per file (`0` behaves as `1`).
    pub concurrency: usize,
}

/// A file that could not be read, parsed or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileError {
    Read {
        path: String,
        msg: String,
    },
    Parse {
        path: String,
        line: Option<usize>,
        msg: String,
    },
    Write {
        path: String,
        msg: String,
    },
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::Read { path, msg } | FileError::Write { path, msg } => {
                write!(f, "{path}: {msg}")
            }
            FileError::Parse {
                path,
                line: Some(line),
                msg,
            } => write!(f, "{path}:{line}: {msg}"),
            FileError::Parse {
                path,
                line: None,
                msg,
            } => write!(f, "{path}: {msg}"),
        }
    }
}

const OVERLAP: &str = "internal: overlapping edits";

/// Read `path`, resolve every target `p` finds (kinds in `on`), apply the
/// edits unless `o.dry_run`/`o.check`, and write only if there is at least
/// one splice. Rows are sorted by line (stable). Findings whose splices
/// overlap another finding's become `unresolved (internal: overlapping
/// edits)` without edits; other findings' edits are still written.
/// Trailing comments at one offset merge into one (R1, `coalesce_comments`).
pub async fn pin_file(
    p: &dyn DynPlugin,
    ctx: &Ctx,
    path: &Path,
    on: &KindSet,
    o: &RunOptions,
) -> Result<Vec<Row>, FileError> {
    let file = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|e| FileError::Read {
        path: file.clone(),
        msg: e.to_string(),
    })?;
    let mut findings = p
        .plan(ctx, path, &text, on, o.concurrency)
        .await
        .map_err(|e| FileError::Parse {
            path: file.clone(),
            line: e.line,
            msg: e.msg,
        })?;
    findings.sort_by_key(|f| f.line);
    mark_overlaps(&mut findings);

    let splices: Vec<Splice> =
        coalesce_comments(findings.iter().flat_map(|f| f.edits.clone()).collect());
    if !o.dry_run && !o.check && !splices.is_empty() {
        std::fs::write(path, apply(&text, &splices)).map_err(|e| FileError::Write {
            path: file.clone(),
            msg: e.to_string(),
        })?;
    }
    Ok(findings
        .into_iter()
        .map(|f| Row {
            plugin: p.id(),
            kind: f.kind,
            file: file.clone(),
            line: f.line + 1,
            reference: f.reference,
            old: f.old,
            new: f.new,
            status: f.status,
            reason: f.reason,
        })
        .collect())
}

/// Two splices overlap when both replace and their ranges intersect, or an
/// insertion lies strictly inside a replacement.
fn overlaps(a: &Splice, b: &Splice) -> bool {
    let inside = |q: usize, r: &Splice| r.start < q && q < r.end;
    match (a.start == a.end, b.start == b.end) {
        (false, false) => a.start < b.end && b.start < a.end,
        (true, false) => inside(a.start, b),
        (false, true) => inside(b.start, a),
        (true, true) => false,
    }
}

/// Mark every finding whose splices overlap another finding's.
fn mark_overlaps(findings: &mut [Finding]) {
    let mut hit = vec![false; findings.len()];
    for i in 0..findings.len() {
        for j in i + 1..findings.len() {
            let clash = findings[i]
                .edits
                .iter()
                .any(|a| findings[j].edits.iter().any(|b| overlaps(a, b)));
            if clash {
                hit[i] = true;
                hit[j] = true;
            }
        }
    }
    for (f, hit) in findings.iter_mut().zip(hit) {
        if hit {
            f.status = Status::Unresolved;
            f.reason = Some(OVERLAP.to_string());
            f.new = None;
            f.edits.clear();
        }
    }
}

#[cfg(all(test, feature = "oci", feature = "gitlab"))]
mod resolver_tests {
    use super::*;
    use crate::gitlab::GitLabClient;
    use crate::oci::ImageRef;
    use crate::oci::registry::RegError;
    use crate::plugin::{KindSpec, Plugin, PluginMeta, ScanError};

    static KINDS: [KindSpec; 1] = [KindSpec {
        id: "x",
        default_on: true,
        help: "",
    }];

    /// One target; `resolve` touches `ctx.oci()` iff `touch_oci`.
    struct Fake {
        touch_oci: bool,
    }

    impl PluginMeta for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn kinds(&self) -> &'static [KindSpec] {
            &KINDS
        }
        fn default_files(&self) -> &'static [&'static str] {
            &[]
        }
    }

    impl Plugin for Fake {
        type Target = ();

        fn scan(&self, _: &Path, _: &str, _: &KindSet) -> Result<Vec<()>, ScanError> {
            Ok(vec![()])
        }

        async fn resolve(&self, ctx: &Ctx, _: &str, _: &()) -> Finding {
            if self.touch_oci {
                let _ = ctx.oci();
            }
            Finding {
                line: 0,
                kind: "x",
                reference: "r".into(),
                old: "r".into(),
                new: None,
                status: Status::Ok,
                reason: None,
                edits: vec![],
                follow_up: None,
            }
        }
    }

    fn env() -> ResolverEnv {
        ResolverEnv {
            gitlab_host: "gitlab.com".into(),
            ..Default::default()
        }
    }

    async fn run_fake(test: &str, fake: &Fake, ctx: &Ctx) {
        let dir = PathBuf::from(format!(".tmp/run-resolvers-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.yml");
        std::fs::write(&p, "x: r\n").unwrap();
        let o = RunOptions {
            dry_run: false,
            check: false,
            concurrency: 4,
        };
        pin_file(fake, ctx, &p, &KindSet::defaults(&KINDS), &o)
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn no_resolver_needed_builds_none() {
        let ctx = Ctx::new(Default::default(), 0, env());
        run_fake("none", &Fake { touch_oci: false }, &ctx).await;
        assert_eq!(ctx.resolvers_built(), Vec::<&str>::new());
    }

    #[tokio::test]
    async fn oci_target_builds_only_oci() {
        let ctx = Ctx::new(Default::default(), 0, env());
        run_fake("oci", &Fake { touch_oci: true }, &ctx).await;
        assert_eq!(ctx.resolvers_built(), ["oci"]);
    }

    #[tokio::test]
    async fn construction_error_is_cached() {
        let ctx = Ctx::new(Default::default(), 0, env()).with_oci(Err("ca: unreadable".into()));
        assert_eq!(ctx.oci().err(), Some("ca: unreadable"));
        assert_eq!(ctx.oci().err(), Some("ca: unreadable"));
        let r = ImageRef::parse("alpine:3.20").unwrap();
        assert!(matches!(
            ctx.oci_digest(&r).await,
            Err(RegError::Unresolved(m)) if m == "ca: unreadable"
        ));
        assert_eq!(ctx.resolvers_built(), ["oci"]);
    }

    #[tokio::test]
    async fn gitlab_resolve_errors_without_request() {
        let client = GitLabClient::new("http://127.0.0.1:9".into(), None, false);
        let ctx = Ctx::new(Default::default(), 0, env()).with_gitlab("gitlab.com", Ok(client));
        assert_eq!(
            ctx.gitlab_resolve(Some("example.org"), "g/p", "main").await,
            Err("gitlab: no client for example.org".to_string())
        );
        let ctx =
            Ctx::new(Default::default(), 0, env()).with_gitlab("gitlab.com", Err("ca: x".into()));
        assert_eq!(
            ctx.gitlab_resolve(None, "g/p", "main").await,
            Err("ca: x".to_string())
        );
    }
}
