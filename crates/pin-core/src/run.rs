//! One file through one plugin (spec §6.2/§6.3): read, plan (scan → bounded
//! resolve), overlap check, write, rows; [`run`] = every routed file in one
//! pass. `Ctx` carries the run's policy.

#[cfg(any(feature = "oci", feature = "gitlab"))]
use std::cell::{OnceCell, RefCell};
#[cfg(any(feature = "oci", feature = "gitlab", feature = "test-support"))]
use std::collections::HashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
#[cfg(any(feature = "oci", feature = "gitlab"))]
use std::hash::Hash;
use std::path::{Path, PathBuf};
#[cfg(any(feature = "oci", feature = "gitlab"))]
use std::rc::Rc;

use crate::edit::{Splice, apply, coalesce_comments};
#[cfg(feature = "gitlab")]
use crate::gitlab::{GitLabClient, GitLabHosts};
#[cfg(feature = "oci")]
use crate::oci::ImageRef;
#[cfg(feature = "oci")]
use crate::oci::registry::{Creds, RegError, RegistryClient};
use crate::plugin::{DynPlugin, Finding, Selection};
use crate::report::{ErrorEntry, Report, Row, Status};
use crate::secret::{Redacted, redact};

/// `release_age.unknown` (spec §6.8): what the release-age guard does when
/// a registry has no timestamps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgePolicy {
    /// Unknown age passes the guard (today's no-op for non-docker.io).
    #[default]
    Allow,
    /// Unknown age blocks an update (skipped, `release age unknown (…)`).
    Block,
}

/// `--update` policy for one run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdatePolicy {
    pub enabled: bool,
    pub allow_major: bool,
    /// Release-age guard in days (0 = off).
    pub min_age_days: u32,
    /// The guard for hosts without timestamps (every host but docker.io).
    pub unknown_age: AgePolicy,
}

/// The credential one GitLab host gets (D31, D37). Pure data, no client.
#[derive(Clone, PartialEq, Eq)]
pub enum GitlabAuth {
    Anonymous,
    /// `GITLAB_TOKEN`, sent as `PRIVATE-TOKEN`.
    Private(String),
    /// `CI_JOB_TOKEN`, sent as `JOB-TOKEN`.
    Job(String),
}

impl GitlabAuth {
    /// `(token, is_job_token)`, the `GitLabClient::try_new` arguments.
    pub fn into_parts(self) -> (Option<String>, bool) {
        match self {
            GitlabAuth::Anonymous => (None, false),
            GitlabAuth::Private(t) => (Some(t), false),
            GitlabAuth::Job(t) => (Some(t), true),
        }
    }
}

impl fmt::Debug for GitlabAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitlabAuth::Anonymous => f.write_str("Anonymous"),
            GitlabAuth::Private(_) => f.debug_tuple("Private").field(&Redacted).finish(),
            GitlabAuth::Job(_) => f.debug_tuple("Job").field(&Redacted).finish(),
        }
    }
}

/// The GitLab side of [`ResolverEnv`]: the primary host, the other hosts
/// that resolve, and the tokens with the host they bind to (T-D12).
#[derive(Clone, PartialEq, Eq)]
pub struct GitlabEnv {
    /// Includes resolve here; `GITLAB_TOKEN` is sent only here (D31).
    pub primary: String,
    /// More hosts that resolve (anonymously unless bound below).
    pub hosts: BTreeSet<String>,
    pub gitlab_token: Option<String>,
    pub ci_job_token: Option<String>,
    /// `CI_JOB_TOKEN` is sent only to this host (D37).
    pub ci_server_host: Option<String>,
}

impl Default for GitlabEnv {
    /// Primary `gitlab.com`, no other host, no token.
    fn default() -> Self {
        Self {
            primary: "gitlab.com".to_string(),
            hosts: BTreeSet::new(),
            gitlab_token: None,
            ci_job_token: None,
            ci_server_host: None,
        }
    }
}

impl fmt::Debug for GitlabEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitlabEnv")
            .field("primary", &self.primary)
            .field("hosts", &self.hosts)
            .field("gitlab_token", &redact(&self.gitlab_token))
            .field("ci_job_token", &redact(&self.ci_job_token))
            .field("ci_server_host", &self.ci_server_host)
            .finish()
    }
}

/// `vars[name]` unless missing or empty.
fn non_empty(vars: &BTreeMap<String, String>, name: &str) -> Option<String> {
    vars.get(name).filter(|v| !v.is_empty()).cloned()
}

/// A GitLab host name as configured or taken from the environment (the
/// primary host, `gitlab-hosts` entries, `CI_SERVER_HOST`): `name[:port]`
/// with `name` = dot-separated labels of ASCII letters, digits and `-`
/// (no empty label, no label starting or ending with `-`) and `port` =
/// 1..=65535. So no scheme, path, userinfo (`gitlab.com@evil.example`),
/// query or fragment (review-2 L6, M5d final M1/N5). The API base is
/// `https://{host}/api/v4`.
pub fn valid_gitlab_host(h: &str) -> bool {
    let (name, port) = match h.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (h, None),
    };
    let label = |l: &str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let port_ok = |p: &str| {
        !p.is_empty()
            && p.chars().all(|c| c.is_ascii_digit())
            && p.parse::<u16>().is_ok_and(|n| n > 0)
    };
    name.split('.').all(label) && port.is_none_or(port_ok)
}

impl GitlabEnv {
    /// `primary` (the toolkit passes CLI > `PIN_TOOLKIT_GITLAB_HOST`), else
    /// `CI_SERVER_HOST`, else `gitlab.com`; tokens from `GITLAB_TOKEN` and
    /// `CI_JOB_TOKEN`. Empty values count as unset. A `CI_SERVER_HOST` that
    /// is not a [`valid_gitlab_host`] is an error, never a fallback: it
    /// would be the default primary and the job token's host (M5d final M1).
    pub fn from_vars(
        vars: &BTreeMap<String, String>,
        primary: Option<String>,
        hosts: BTreeSet<String>,
    ) -> Result<Self, String> {
        let ci_server_host = non_empty(vars, "CI_SERVER_HOST");
        if let Some(h) = &ci_server_host
            && !valid_gitlab_host(h)
        {
            return Err(format!("CI_SERVER_HOST: invalid GitLab host `{h}`"));
        }
        let primary = primary
            .filter(|p| !p.is_empty())
            .or_else(|| ci_server_host.clone())
            .unwrap_or_else(|| "gitlab.com".to_string());
        Ok(Self {
            primary,
            hosts,
            gitlab_token: non_empty(vars, "GITLAB_TOKEN"),
            ci_job_token: non_empty(vars, "CI_JOB_TOKEN"),
            ci_server_host,
        })
    }

    /// `host` resolves: the primary or one of `hosts`.
    pub fn enabled(&self, host: &str) -> bool {
        host == self.primary || self.hosts.contains(host)
    }

    /// Every host that gets a client: `{primary} ∪ hosts`.
    pub fn resolve_set(&self) -> BTreeSet<String> {
        let mut set = self.hosts.clone();
        set.insert(self.primary.clone());
        set
    }

    /// `GITLAB_TOKEN` for the primary; else `CI_JOB_TOKEN` for
    /// `CI_SERVER_HOST`; else anonymous (T-D12).
    pub fn auth_for(&self, host: &str) -> GitlabAuth {
        if host == self.primary
            && let Some(t) = &self.gitlab_token
        {
            return GitlabAuth::Private(t.clone());
        }
        if self.ci_server_host.as_deref() == Some(host)
            && let Some(t) = &self.ci_job_token
        {
            return GitlabAuth::Job(t.clone());
        }
        GitlabAuth::Anonymous
    }
}

/// The registry credential variables read from the env map (PR-M2).
pub const REGISTRY_VARS: [&str; 3] = ["DOCKERHUB_USER", "DOCKERHUB_TOKEN", "GITHUB_TOKEN"];

/// [`REGISTRY_VARS`] present in the env map (empty values dropped).
/// `Debug` prints the names only (T-D29).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RegistryEnv(BTreeMap<String, String>);

impl RegistryEnv {
    fn from_vars(vars: &BTreeMap<String, String>) -> Self {
        Self(
            REGISTRY_VARS
                .iter()
                .filter_map(|v| non_empty(vars, v).map(|val| (v.to_string(), val)))
                .collect(),
        )
    }

    #[cfg_attr(not(feature = "oci"), allow(dead_code))]
    fn get(&self, var: &str) -> Option<String> {
        self.0.get(var).cloned()
    }
}

impl fmt::Debug for RegistryEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|k| (k, Redacted)))
            .finish()
    }
}

/// Test hook: base-URL overrides of the resolvers (`test-support`, D38).
#[cfg(feature = "test-support")]
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct BaseUrls {
    /// Registry host → base URL (`RegistryClient` `base_urls`).
    pub registry: HashMap<String, String>,
    /// Docker Hub API base (`RegistryClient::with_hub_base`).
    pub hub: Option<String>,
    /// GitLab host → API base (default `https://{host}/api/v4`).
    pub gitlab: BTreeMap<String, String>,
}

/// What resolvers are built from: read once per run, from the env map the
/// front-end passes (no process-env reads below the binary edge).
#[derive(Debug, Clone, Default)]
pub struct ResolverEnv {
    /// Home directory (`~/.docker/config.json` credentials).
    pub home: Option<PathBuf>,
    pub gitlab: GitlabEnv,
    pub registry: RegistryEnv,
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub base_urls: BaseUrls,
}

impl ResolverEnv {
    /// `home` = `HOME` (empty = unset: never a cwd-relative
    /// `.docker/config.json`), `gitlab` = [`GitlabEnv::from_vars`] (its
    /// error is this one's), `registry` = [`REGISTRY_VARS`] from `vars`.
    pub fn from_vars(
        vars: &BTreeMap<String, String>,
        primary: Option<String>,
        hosts: BTreeSet<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            home: non_empty(vars, "HOME").map(PathBuf::from),
            gitlab: GitlabEnv::from_vars(vars, primary, hosts)?,
            registry: RegistryEnv::from_vars(vars),
            #[cfg(feature = "test-support")]
            base_urls: BaseUrls::default(),
        })
    }

    /// Registry credentials: env-map variables over `~/.docker/config.json`
    /// under `home`; never the process env (PR-M2).
    #[cfg(feature = "oci")]
    pub(crate) fn creds(&self) -> Creds {
        Creds::load_from_env(self.home.as_deref(), |v| self.registry.get(v))
    }

    /// [`ResolverEnv::from_vars`] over the process environment (UTF-8
    /// variables only) with `primary` and no other GitLab host: for the
    /// pin-ci driver and the live/oracle tests. A process-env reader below
    /// the binary edge: test helpers only (PR-L9). Panics on an invalid
    /// `CI_SERVER_HOST`.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn from_process(primary: &str) -> Self {
        let vars: BTreeMap<String, String> = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect();
        Self::from_vars(&vars, Some(primary.to_string()), BTreeSet::new())
            .unwrap_or_else(|e| panic!("{e}"))
    }
}

/// One resolver entry point's results for one run (§6.7, D19), keyed by
/// lookup. Values include errors, so one key cannot get two outcomes; the
/// `OnceCell` coalesces concurrent lookups of the same key. `Ctx` is
/// single-threaded (futures are joined, never spawned): `RefCell` + `Rc`.
#[cfg(any(feature = "oci", feature = "gitlab"))]
struct Memo<K, V>(RefCell<HashMap<K, Rc<tokio::sync::OnceCell<V>>>>);

#[cfg(any(feature = "oci", feature = "gitlab"))]
impl<K, V> Default for Memo<K, V> {
    fn default() -> Self {
        Self(RefCell::new(HashMap::new()))
    }
}

#[cfg(any(feature = "oci", feature = "gitlab"))]
impl<K: Eq + Hash, V: Clone> Memo<K, V> {
    /// The result for `k`: computed by `f` on the first call, shared by
    /// every concurrent and later call.
    async fn get<F: Future<Output = V>>(&self, k: K, f: impl FnOnce() -> F) -> V {
        // The borrow ends before the await.
        let cell = Rc::clone(self.0.borrow_mut().entry(k).or_default());
        cell.get_or_init(f).await.clone()
    }
}

/// The memos of the `Ctx` entry points (§6.7 keys).
#[cfg(any(feature = "oci", feature = "gitlab"))]
#[derive(Default)]
struct Memos {
    /// `(host, api_repo, effective_tag)` → manifest digest.
    #[cfg(feature = "oci")]
    digest: Memo<(String, String, String), Result<String, RegError>>,
    /// `(host, api_repo)` → tag list.
    #[cfg(feature = "oci")]
    tags: Memo<(String, String), Result<Vec<String>, RegError>>,
    /// `(host, api_repo)` → tag push times.
    #[cfg(feature = "oci")]
    times: Memo<(String, String), Result<BTreeMap<String, i64>, RegError>>,
    /// `(host, project, ref)` → commit SHA (`host` = the primary when the
    /// caller passes `None`).
    #[cfg(feature = "gitlab")]
    gitlab: Memo<(String, String, String), Result<String, String>>,
}

/// Per-run context: policy, clock, the per-run memo and lazily built,
/// feature-gated resolvers (D36). Plugins resolve only through its entry
/// points (D38).
pub struct Ctx {
    pub update: UpdatePolicy,
    pub now_unix: i64,
    // Read only by the resolver builders; never exposed to plugins (D37/D38).
    #[cfg_attr(not(any(feature = "oci", feature = "gitlab")), allow(dead_code))]
    env: ResolverEnv,
    #[cfg(any(feature = "oci", feature = "gitlab"))]
    memo: Memos,
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
            #[cfg(any(feature = "oci", feature = "gitlab"))]
            memo: Memos::default(),
            #[cfg(feature = "oci")]
            oci: OnceCell::new(),
            #[cfg(feature = "gitlab")]
            gitlab: OnceCell::new(),
        }
    }

    /// GitLab `host` resolves (the primary or one of `gitlab-hosts`): the
    /// one host set plugins gate on (review L9).
    pub fn gitlab_enabled(&self, host: &str) -> bool {
        self.env.gitlab.enabled(host)
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

    /// The registry client, built from `env` on first use (credentials from
    /// the env map, PR-M2); a construction error is cached.
    #[cfg(feature = "oci")]
    pub(crate) fn oci(&self) -> Result<&RegistryClient, &str> {
        self.oci
            .get_or_init(|| {
                #[cfg(feature = "test-support")]
                let (bases, hub) = (
                    self.env.base_urls.registry.clone(),
                    self.env.base_urls.hub.clone(),
                );
                #[cfg(not(feature = "test-support"))]
                let (bases, hub) = (HashMap::new(), None::<String>);
                let client = RegistryClient::try_new(self.env.creds(), bases)?;
                Ok(match hub {
                    Some(h) => client.with_hub_base(&h),
                    None => client,
                })
            })
            .as_ref()
            .map_err(String::as_str)
    }

    /// Preset the registry client (or its construction error).
    #[cfg(all(feature = "oci", any(test, feature = "test-support")))]
    #[doc(hidden)]
    pub fn with_oci(self, c: Result<RegistryClient, String>) -> Self {
        Self {
            oci: OnceCell::from(c),
            ..self
        }
    }

    /// The GitLab clients, one per `env.gitlab.resolve_set()` host, each
    /// with the credential [`GitlabEnv::auth_for`] binds to it (D31, D37);
    /// built on first use, a construction error (`gitlab: {host}: {e}`) is
    /// cached.
    #[cfg(feature = "gitlab")]
    pub(crate) fn gitlab(&self) -> Result<&GitLabHosts, &str> {
        self.gitlab
            .get_or_init(|| {
                let g = &self.env.gitlab;
                let mut clients = BTreeMap::new();
                for host in g.resolve_set() {
                    let (token, job) = g.auth_for(&host).into_parts();
                    let client = GitLabClient::try_new(self.gitlab_base(&host), token, job)
                        .map_err(|e| format!("gitlab: {host}: {e}"))?;
                    clients.insert(host, client);
                }
                Ok(GitLabHosts::new(&g.primary, clients))
            })
            .as_ref()
            .map_err(String::as_str)
    }

    /// The API base of GitLab `host`: the `test-support` override, else
    /// `https://{host}/api/v4`.
    #[cfg(feature = "gitlab")]
    fn gitlab_base(&self, host: &str) -> String {
        #[cfg(feature = "test-support")]
        if let Some(base) = self.env.base_urls.gitlab.get(host) {
            return base.clone();
        }
        format!("https://{host}/api/v4")
    }

    /// Preset the client of the primary GitLab host (or its construction
    /// error); `primary_host` also becomes `env.gitlab.primary`.
    #[cfg(all(feature = "gitlab", any(test, feature = "test-support")))]
    #[doc(hidden)]
    pub fn with_gitlab(mut self, primary_host: &str, c: Result<GitLabClient, String>) -> Self {
        self.env.gitlab.primary = primary_host.to_string();
        Self {
            gitlab: OnceCell::from(c.map(|c| GitLabHosts::single(primary_host, c))),
            ..self
        }
    }

    /// Manifest digest of `r`; a construction error is `RegError::Unresolved`.
    /// Memoised per `(host, api_repo, effective_tag)` (§6.7).
    #[cfg(feature = "oci")]
    pub async fn oci_digest(&self, r: &ImageRef) -> Result<String, RegError> {
        let key = (
            r.host_str().to_string(),
            r.api_repo(),
            r.effective_tag().to_string(),
        );
        self.memo
            .digest
            .get(key, || async {
                self.oci()
                    .map_err(|e| RegError::Unresolved(e.to_string()))?
                    .digest(r)
                    .await
            })
            .await
    }

    /// Tag list of `r`'s repository. Memoised per `(host, api_repo)`.
    #[cfg(feature = "oci")]
    pub async fn oci_tags(&self, r: &ImageRef) -> Result<Vec<String>, RegError> {
        let key = (r.host_str().to_string(), r.api_repo());
        self.memo
            .tags
            .get(key, || async {
                self.oci()
                    .map_err(|e| RegError::Unresolved(e.to_string()))?
                    .list_tags(r)
                    .await
            })
            .await
    }

    /// Tag push times (unix seconds) of `r`'s repository (Docker Hub API).
    /// Memoised per `(host, api_repo)`.
    #[cfg(feature = "oci")]
    pub async fn oci_tag_times(&self, r: &ImageRef) -> Result<BTreeMap<String, i64>, RegError> {
        let key = (r.host_str().to_string(), r.api_repo());
        self.memo
            .times
            .get(key, || async {
                self.oci()
                    .map_err(|e| RegError::Unresolved(e.to_string()))?
                    .tag_timestamps(r)
                    .await
            })
            .await
    }

    /// Resolve `git_ref` in `project` on GitLab `host` (`None` = the primary
    /// host) to a commit SHA. Errors are the client's texts, the cached
    /// construction error, or `gitlab: no client for {host}`. Memoised per
    /// `(host, project, ref)` with `None` keyed as the primary host (§6.7).
    #[cfg(feature = "gitlab")]
    pub async fn gitlab_resolve(
        &self,
        host: Option<&str>,
        project: &str,
        git_ref: &str,
    ) -> Result<String, String> {
        let hosts = self.gitlab().map_err(str::to_string)?;
        let host = host.unwrap_or(hosts.primary_host());
        let key = (host.to_string(), project.to_string(), git_ref.to_string());
        self.memo
            .gitlab
            .get(key, || async {
                let client = hosts
                    .get(host)
                    .ok_or_else(|| format!("gitlab: no client for {host}"))?;
                client
                    .resolve(project, git_ref)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
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

impl FileError {
    /// The report entry for this error, the file shown as `display`; the
    /// message is the variant's text without the path (spec §6.3).
    pub fn entry(&self, display: &str) -> ErrorEntry {
        let (line, msg) = match self {
            FileError::Read { msg, .. } | FileError::Write { msg, .. } => (None, msg),
            FileError::Parse { line, msg, .. } => (*line, msg),
        };
        ErrorEntry {
            file: display.to_string(),
            line,
            message: msg.clone(),
        }
    }
}

/// One file of a run: the plugin that claimed it, what that plugin plans,
/// the path for I/O and the path as shown in the report.
pub struct Route<'a> {
    pub plugin: &'a dyn DynPlugin,
    pub sel: &'a Selection,
    pub path: PathBuf,
    pub display: String,
}

/// One pass over `routes` (spec §6.3), sequentially in the given order:
/// [`pin_file`] each; rows' `file` is the route's display path; a file
/// error becomes an [`ErrorEntry`] under the route's plugin and the next
/// file still runs. Every routed plugin has a report entry.
pub async fn run(routes: &[Route<'_>], ctx: &Ctx, o: RunOptions) -> Report {
    let mut rows = Vec::new();
    let mut ran: Vec<&'static str> = Vec::new();
    let mut errors = Vec::new();
    for r in routes {
        let id = r.plugin.id();
        if !ran.contains(&id) {
            ran.push(id);
        }
        match pin_file(r.plugin, ctx, &r.path, r.sel, &o).await {
            Ok(file_rows) => rows.extend(file_rows.into_iter().map(|row| Row {
                file: r.display.clone(),
                ..row
            })),
            Err(e) => errors.push((id, e.entry(&r.display))),
        }
    }
    Report::assemble(rows, &ran, errors)
}

const OVERLAP: &str = "internal: overlapping edits";

/// Read `path`, resolve every target `p` finds (kinds in `sel`, minus the
/// targets its `ignore` patterns match: no row, never resolved), apply the
/// edits unless `o.dry_run`/`o.check`, and write only if there is at least
/// one splice. Rows are sorted by line (stable). Findings whose splices
/// overlap another finding's become `unresolved (internal: overlapping
/// edits)` without edits or follow-up; other findings' edits are still
/// written. Edits of findings that are neither pinned nor updated are
/// dropped (review L2).
/// Trailing comments at one offset merge into one (R1, `coalesce_comments`).
pub async fn pin_file(
    p: &dyn DynPlugin,
    ctx: &Ctx,
    path: &Path,
    sel: &Selection,
    o: &RunOptions,
) -> Result<Vec<Row>, FileError> {
    let file = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|e| FileError::Read {
        path: file.clone(),
        msg: e.to_string(),
    })?;
    let mut findings = p
        .plan(ctx, path, &text, sel, o.concurrency)
        .await
        .map_err(|e| FileError::Parse {
            path: file.clone(),
            line: e.line,
            msg: e.msg,
        })?;
    findings.sort_by_key(|f| f.line);
    // Only pinned/updated findings edit the file (review L2); dropped before
    // the overlap check so a stray edit cannot mark a real pin.
    for f in &mut findings {
        if !matches!(f.status, Status::Pinned | Status::Updated) {
            f.edits.clear();
        }
    }
    mark_overlaps(&mut findings);

    let splices = ordered_splices(&findings);
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
            follow_up: f.follow_up,
        })
        .collect())
}

/// Every finding's splices, findings stably ordered by `(line, first edit
/// offset)` so trailing comments merge in byte order whatever order the
/// plugin found the references in (review L1), then `coalesce_comments`.
/// Rows keep finding order; only the splice order is touched.
fn ordered_splices(findings: &[Finding]) -> Vec<Splice> {
    let mut by_position: Vec<&Finding> = findings.iter().collect();
    by_position.sort_by_key(|f| {
        let first = f.edits.iter().map(|s| s.start).min();
        (f.line, first.unwrap_or(usize::MAX))
    });
    coalesce_comments(
        by_position
            .into_iter()
            .flat_map(|f| f.edits.iter().cloned())
            .collect(),
    )
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

/// Mark every finding whose splices overlap another finding's (its edits
/// and follow-up are dropped).
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
            f.follow_up = None;
        }
    }
}

#[cfg(all(test, feature = "oci", feature = "gitlab"))]
mod resolver_tests {
    use super::*;
    use crate::gitlab::GitLabClient;
    use crate::oci::ImageRef;
    use crate::oci::registry::RegError;
    use crate::plugin::{KindSet, KindSpec, Plugin, PluginMeta, ScanError, Source};

    static KINDS: [KindSpec; 1] = [KindSpec {
        id: "x",
        default_on: true,
        help: "",
    }];

    /// One target; `resolve` touches `ctx.oci()` iff `touch_oci` and
    /// `ctx.gitlab()` iff `touch_gitlab`.
    struct Fake {
        touch_oci: bool,
        touch_gitlab: bool,
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

        async fn resolve(&self, ctx: &Ctx, _: &Source<'_>, _: &()) -> Finding {
            if self.touch_oci {
                let _ = ctx.oci();
            }
            if self.touch_gitlab {
                let _ = ctx.gitlab();
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
        ResolverEnv::default()
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
        pin_file(fake, ctx, &p, &KindSet::defaults(&KINDS).into(), &o)
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn no_resolver_needed_builds_none() {
        let ctx = Ctx::new(Default::default(), 0, env());
        run_fake(
            "none",
            &Fake {
                touch_oci: false,
                touch_gitlab: false,
            },
            &ctx,
        )
        .await;
        assert_eq!(ctx.resolvers_built(), Vec::<&str>::new());
    }

    #[tokio::test]
    async fn oci_target_builds_only_oci() {
        let ctx = Ctx::new(Default::default(), 0, env());
        run_fake(
            "oci",
            &Fake {
                touch_oci: true,
                touch_gitlab: false,
            },
            &ctx,
        )
        .await;
        assert_eq!(ctx.resolvers_built(), ["oci"]);
    }

    #[tokio::test]
    async fn gitlab_target_builds_only_gitlab() {
        let ctx = Ctx::new(Default::default(), 0, env());
        run_fake(
            "gitlab",
            &Fake {
                touch_oci: false,
                touch_gitlab: true,
            },
            &ctx,
        )
        .await;
        assert_eq!(ctx.resolvers_built(), ["gitlab"]);
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

    /// PR-M2: registry credentials come from the env map, never the process.
    /// No `HOME` in the map ⇒ no docker config is read (deterministic).
    #[test]
    fn registry_creds_from_vars() {
        use crate::oci::registry::HostCreds;
        let vars = |kv: &[(&str, &str)]| -> BTreeMap<String, String> {
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let env =
            ResolverEnv::from_vars(&vars(&[("GITHUB_TOKEN", "x")]), None, BTreeSet::new()).unwrap();
        assert_eq!(
            env.creds().for_host("ghcr.io"),
            Some(&HostCreds::Bearer { token: "x".into() })
        );
        let env = ResolverEnv::from_vars(
            &vars(&[("DOCKERHUB_USER", "u"), ("DOCKERHUB_TOKEN", "t")]),
            None,
            BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(
            env.creds().for_host("docker.io"),
            Some(&HostCreds::Basic {
                user: "u".into(),
                password: "t".into()
            })
        );
        let env = ResolverEnv::from_vars(&BTreeMap::new(), None, BTreeSet::new()).unwrap();
        assert_eq!(env.creds().for_host("ghcr.io"), None);
        // An empty HOME is unset, never a cwd-relative `.docker/config.json`
        // (M5d review-1 L4).
        let home = |h: &str| {
            ResolverEnv::from_vars(&vars(&[("HOME", h)]), None, BTreeSet::new())
                .unwrap()
                .home
        };
        assert_eq!(home(""), None);
        assert_eq!(home("/h"), Some(PathBuf::from("/h")));
    }

    /// PR-M5 (P-redact): no token-holding value prints its secret in `Debug`.
    #[test]
    fn debug_redacts_tokens() {
        use crate::oci::registry::HostCreds;
        const S: &str = "s3cr3t";
        let vars: BTreeMap<String, String> = [
            "GITLAB_TOKEN",
            "CI_JOB_TOKEN",
            "DOCKERHUB_USER",
            "DOCKERHUB_TOKEN",
            "GITHUB_TOKEN",
        ]
        .into_iter()
        .map(|k| (k.to_string(), S.to_string()))
        .chain([("CI_SERVER_HOST".to_string(), "gitlab.com".to_string())])
        .collect();
        let env = ResolverEnv::from_vars(&vars, None, BTreeSet::new()).unwrap();
        assert!(env.gitlab.gitlab_token.is_some() && env.gitlab.ci_job_token.is_some());
        let client =
            GitLabClient::try_new("http://127.0.0.1:9".into(), Some(S.into()), true).unwrap();
        let printed = [
            format!("{:?}", env.gitlab),
            format!("{:?}", GitlabAuth::Private(S.into())),
            format!("{:?}", GitlabAuth::Job(S.into())),
            format!("{:?}", env),
            format!("{:?}", client),
            format!(
                "{:?}",
                HostCreds::Basic {
                    user: "u".into(),
                    password: S.into()
                }
            ),
            format!("{:?}", HostCreds::Bearer { token: S.into() }),
            format!(
                "{:?}",
                RegistryClient::new(
                    Creds::none().with_host("ghcr.io", HostCreds::Bearer { token: S.into() }),
                    HashMap::new()
                )
            ),
        ];
        for p in printed {
            assert!(!p.contains(S), "secret leaked: {p}");
            assert!(p.contains("<redacted>"), "{p}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gitlab_env(
        primary: &str,
        ci_server_host: Option<&str>,
        gitlab_token: Option<&str>,
        ci_job_token: Option<&str>,
    ) -> GitlabEnv {
        let vars: BTreeMap<String, String> = [
            ("CI_SERVER_HOST", ci_server_host),
            ("GITLAB_TOKEN", gitlab_token),
            ("CI_JOB_TOKEN", ci_job_token),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k.to_string(), v.to_string())))
        .collect();
        GitlabEnv::from_vars(&vars, Some(primary.to_string()), BTreeSet::new()).unwrap()
    }

    /// D31 + D37 (T-D12): `GITLAB_TOKEN` only to the primary, `CI_JOB_TOKEN`
    /// only to `CI_SERVER_HOST`, `GITLAB_TOKEN` wins when both apply.
    #[test]
    fn auth_for_binds_tokens() {
        use GitlabAuth::{Anonymous, Job, Private};
        let (com, ex) = ("gitlab.com", "gitlab.example.com");
        let pt = || Private("pt".into());
        let table = [
            ((com, None, Some("pt"), None, com), pt()),
            ((com, None, Some("pt"), None, ex), Anonymous),
            ((ex, Some(ex), None, Some("jt"), ex), Job("jt".into())),
            ((com, Some(ex), None, Some("jt"), com), Anonymous),
            ((com, None, None, Some("jt"), com), Anonymous), // the pre-M5d leak
            ((ex, Some(ex), Some("pt"), Some("jt"), ex), pt()),
        ];
        for ((primary, server, token, job, host), want) in table {
            let env = gitlab_env(primary, server, token, job);
            assert_eq!(
                env.auth_for(host),
                want,
                "{primary} {server:?} {token:?} {job:?} → {host}"
            );
        }

        // Primary: explicit > CI_SERVER_HOST > gitlab.com; empty = unset.
        let vars = |kv: &[(&str, &str)]| -> BTreeMap<String, String> {
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let primary = |vars: &BTreeMap<String, String>, p: Option<&str>| {
            GitlabEnv::from_vars(vars, p.map(str::to_string), BTreeSet::new())
                .unwrap()
                .primary
        };
        let server = vars(&[("CI_SERVER_HOST", ex)]);
        assert_eq!(primary(&server, Some("h.example")), "h.example");
        assert_eq!(primary(&server, None), ex);
        assert_eq!(primary(&server, Some("")), ex);
        assert_eq!(primary(&BTreeMap::new(), None), com);
        assert_eq!(primary(&vars(&[("CI_SERVER_HOST", "")]), None), com);
        let empty = vars(&[("GITLAB_TOKEN", ""), ("CI_JOB_TOKEN", "")]);
        let env = GitlabEnv::from_vars(&empty, None, BTreeSet::new()).unwrap();
        assert_eq!(env.auth_for(com), Anonymous);
        assert_eq!(env.ci_server_host, None);

        // Enabled hosts and the resolve set: {primary} ∪ hosts.
        let env =
            GitlabEnv::from_vars(&BTreeMap::new(), None, BTreeSet::from([ex.to_string()])).unwrap();
        assert!(env.enabled(com) && env.enabled(ex) && !env.enabled("other.example"));
        assert_eq!(
            env.resolve_set(),
            BTreeSet::from([com.to_string(), ex.to_string()])
        );
    }

    /// Host syntax (review-2 L6, M5d final N5): `name[:port]`, labels of
    /// letters, digits and inner `-`, port 1..=65535.
    #[test]
    fn valid_gitlab_host_syntax() {
        let good = [
            "gitlab.com",
            "gitlab.example.com:8443",
            "a",
            "x-1.example",
            "127.0.0.1:9",
            "h:1",
            "h:65535",
            "GitLab.Example.COM",
        ];
        for h in good {
            assert!(valid_gitlab_host(h), "{h} must be valid");
        }
        let bad = [
            "",
            ".",
            "-",
            "..",
            "a..b",
            ".gitlab.com",
            "gitlab.com.",
            "-gitlab.com",
            "gitlab-.com",
            "a.-b.c",
            "gitlab.com@evil.example",
            "gitlab.com@127.0.0.1:9",
            "https://x",
            "a b",
            "h?x",
            "h#x",
            "h/x",
            "h:",
            "h:x",
            "h:0",
            "h:65536",
            "h:99999999999",
            "h:+1",
            "h:1:2",
            ":443",
            "ä.example",
        ];
        for h in bad {
            assert!(!valid_gitlab_host(h), "{h} must be invalid");
        }
    }

    /// M5d final M1: an invalid `CI_SERVER_HOST` fails loud (it would be the
    /// default primary and the job token's host), even when an explicit
    /// primary is set; an empty one is unset.
    #[test]
    fn invalid_ci_server_host_is_an_error() {
        let bad = "gitlab.com@127.0.0.1:9";
        let vars: BTreeMap<String, String> = [
            ("CI_SERVER_HOST", bad),
            ("CI_JOB_TOKEN", "jt"),
            ("GITLAB_TOKEN", "pt"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let want = Err(format!("CI_SERVER_HOST: invalid GitLab host `{bad}`"));
        assert_eq!(GitlabEnv::from_vars(&vars, None, BTreeSet::new()), want);
        assert_eq!(
            GitlabEnv::from_vars(&vars, Some("gitlab.com".into()), BTreeSet::new()),
            want
        );
        assert_eq!(
            ResolverEnv::from_vars(&vars, None, BTreeSet::new()).map(|e| e.gitlab),
            want
        );
    }
}
