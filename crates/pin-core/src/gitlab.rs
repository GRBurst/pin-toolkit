//! GitLab REST v4 client: resolve a project ref (tag, branch, or bare name)
//! to a full 40-hex commit SHA (M3 pinning, plan constraint 7).
//!
//! Resolution flow:
//! - 40-hex lowercase SHA → returned as-is, no API call.
//! - `refs/tags/<t>` → `GET /projects/{id}/repository/tags/{t}`.
//! - `refs/heads/<b>` → the branch path.
//! - Bare name → tag first (GitLab tag-precedence semantics), 404 → branch,
//!   404 → `Unresolved("{project}:{ref} not found (no tag or branch)")`.
//!
//! Auth (plan constraint 6): `GITLAB_TOKEN` is sent as `PRIVATE-TOKEN`;
//! `CI_JOB_TOKEN` is sent as `JOB-TOKEN`. Job-token branch resolution uses
//! the list endpoint `GET /projects/{id}/repository/branches?search={name}`
//! with an EXACT name match — the single-branch endpoint is not in the
//! job-token allowlist (recon §4). PAT/anonymous use the single-branch
//! endpoint.
//!
//! Only successful resolutions are cached, per `(project, ref)`, so
//! repeated refs across files cost one API call.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use tokio::sync::Mutex;

/// Errors from GitLab lookups.
#[derive(Debug)]
pub enum GitLabError {
    /// The ref could not be resolved to a commit SHA: no tag and no branch,
    /// 401/403, 429, another HTTP failure, a malformed 200 body, or a
    /// network error. `Display` is the human-readable reason.
    Unresolved(String),
}

impl fmt::Display for GitLabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitLabError::Unresolved(msg) => write!(f, "{msg}"),
        }
    }
}

/// Outcome of one GitLab API GET (private). The `NotFound` sentinel is
/// distinct from real failures so the bare-name tag→branch fallback can
/// proceed; only the final fallback surfaces `GitLabError`.
#[derive(Debug)]
enum Lookup {
    /// 200 with a valid `commit.id`.
    Commit(String),
    /// 404, or a job-token branch list without an exact-name match.
    NotFound,
    /// Auth, rate-limit, other non-2xx, or malformed 2xx body.
    Error(GitLabError),
}

/// A 200 body without a usable `commit.id` (plan GREEN detail).
fn malformed() -> Lookup {
    Lookup::Error(GitLabError::Unresolved(
        "gitlab api: malformed response".to_string(),
    ))
}

/// Parse a 200 JSON body with a `commit.id` string.
fn parse_commit_id(body: &str) -> Lookup {
    let value = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) => v,
        Err(_) => return malformed(),
    };
    match value
        .get("commit")
        .and_then(|c| c.get("id"))
        .and_then(|id| id.as_str())
    {
        Some(id) => Lookup::Commit(id.to_string()),
        None => malformed(),
    }
}

/// Which endpoint resolves this ref (plan constraint 7).
enum RefKind<'a> {
    Tag(&'a str),
    Branch(&'a str),
    Bare(&'a str),
}

fn classify_ref(ref_name: &str) -> RefKind<'_> {
    if let Some(t) = ref_name.strip_prefix("refs/tags/") {
        RefKind::Tag(t)
    } else if let Some(b) = ref_name.strip_prefix("refs/heads/") {
        RefKind::Branch(b)
    } else {
        RefKind::Bare(ref_name)
    }
}

/// GitLab REST v4 client: ref → commit SHA (M3 pinning). The cache sits
/// behind `Arc`, so clones share it (M5c, P11). `Debug` redacts the token
/// (T-D29).
#[derive(Clone)]
pub struct GitLabClient {
    base_url: String,
    token: Option<String>,
    /// True when the token is a `CI_JOB_TOKEN` (sent as `JOB-TOKEN`, and
    /// resolved via the job-token-safe branch-list endpoint).
    job_token: bool,
    http: reqwest::Client,
    /// Resolved commit SHAs keyed by `(project, ref)`; successful
    /// resolutions only.
    cache: Arc<Mutex<HashMap<(String, String), String>>>,
}

impl fmt::Debug for GitLabClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitLabClient")
            .field("base_url", &self.base_url)
            .field("token", &crate::secret::redact(&self.token))
            .field("job_token", &self.job_token)
            .field("http", &self.http)
            .field("cache", &self.cache)
            .finish()
    }
}

impl GitLabClient {
    /// Client for `base_url` (e.g. `https://gitlab.com/api/v4`). `token` is
    /// sent as `JOB-TOKEN` when `job_token` is set, else as
    /// `PRIVATE-TOKEN`; 30s request timeout. Panics if the HTTP client
    /// cannot be built (test convenience; see [`GitLabClient::try_new`]).
    pub fn new(base_url: String, token: Option<String>, job_token: bool) -> Self {
        Self::try_new(base_url, token, job_token).expect("reqwest client builds")
    }

    /// Like [`GitLabClient::new`], but an HTTP client construction error is
    /// returned as `"http client: {e}"` (review L5).
    pub fn try_new(
        base_url: String,
        token: Option<String>,
        job_token: bool,
    ) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self {
            base_url,
            token,
            job_token,
            http,
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Resolve `ref_name` in `project` to a full 40-hex commit SHA.
    ///
    /// Flow (M3 constraint 7): 40-hex lowercase SHA → as-is, no API call.
    /// `refs/tags/<t>` → tag endpoint; `refs/heads/<b>` → branch; bare name →
    /// tag first, 404 → branch, 404 → `Unresolved("{project}:{ref} not found
    /// (no tag or branch)")`. Successful resolutions are cached per
    /// `(project, ref)`.
    pub async fn resolve(&self, project: &str, ref_name: &str) -> Result<String, GitLabError> {
        if crate::git::is_commit_sha(ref_name) {
            return Ok(ref_name.to_string());
        }
        let key = (project.to_string(), ref_name.to_string());
        {
            let cache = self.cache.lock().await;
            if let Some(sha) = cache.get(&key) {
                return Ok(sha.clone());
            }
        }
        let sha = self.resolve_uncached(project, ref_name).await?;
        self.cache.lock().await.insert(key, sha.clone());
        Ok(sha)
    }

    /// The resolution flow without the cache (plan constraint 7).
    async fn resolve_uncached(&self, project: &str, ref_name: &str) -> Result<String, GitLabError> {
        let not_found = || {
            GitLabError::Unresolved(format!("{project}:{ref_name} not found (no tag or branch)"))
        };
        let lookup = match classify_ref(ref_name) {
            RefKind::Tag(name) => self.fetch_tag(project, name).await,
            RefKind::Branch(name) => self.fetch_branch(project, name).await,
            RefKind::Bare(name) => {
                let tag = self.fetch_tag(project, name).await;
                match tag {
                    Lookup::Commit(sha) => Lookup::Commit(sha),
                    // GitLab tag-precedence semantics: a tag and branch may
                    // share a name — the tag wins; only a 404 falls back.
                    Lookup::NotFound => self.fetch_branch(project, name).await,
                    Lookup::Error(e) => Lookup::Error(e),
                }
            }
        };
        match lookup {
            Lookup::Commit(sha) => Ok(sha),
            Lookup::NotFound => Err(not_found()),
            Lookup::Error(e) => Err(e),
        }
    }

    /// `GET /projects/{id}/repository/tags/{tag}` → `commit.id`.
    async fn fetch_tag(&self, project: &str, tag: &str) -> Lookup {
        let url = format!(
            "{}/projects/{}/repository/tags/{}",
            self.base(),
            url_encode(project),
            url_encode(tag)
        );
        self.get_commit_id(&url).await
    }

    /// Branch → `commit.id`: the single-branch endpoint for PAT/anonymous;
    /// the job-token-safe list endpoint when `job_token` is set.
    async fn fetch_branch(&self, project: &str, branch: &str) -> Lookup {
        if self.job_token {
            return self.fetch_branch_via_list(project, branch).await;
        }
        let url = format!(
            "{}/projects/{}/repository/branches/{}",
            self.base(),
            url_encode(project),
            url_encode(branch)
        );
        self.get_commit_id(&url).await
    }

    /// Job-token branch path (plan constraint 6):
    /// `GET /projects/{id}/repository/branches?search={name}` with an EXACT
    /// name match — the single-branch endpoint is not in the job-token
    /// allowlist (recon §4), and `search=` is a substring filter, so the
    /// near-miss `main-x` must not win over `main`.
    async fn fetch_branch_via_list(&self, project: &str, branch: &str) -> Lookup {
        let url = format!(
            "{}/projects/{}/repository/branches?search={}",
            self.base(),
            url_encode(project),
            url_encode(branch)
        );
        let body = match self.get_text(&url).await {
            Ok(Some(text)) => text,
            Ok(None) => return Lookup::NotFound,
            Err(e) => return e,
        };
        let value = match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(v) => v,
            Err(_) => return malformed(),
        };
        let Some(entries) = value.as_array() else {
            return malformed();
        };
        for entry in entries {
            if entry.get("name").and_then(|n| n.as_str()) != Some(branch) {
                continue; // exact name match only
            }
            let id = entry
                .get("commit")
                .and_then(|c| c.get("id"))
                .and_then(|i| i.as_str());
            return match id {
                Some(id) => Lookup::Commit(id.to_string()),
                None => malformed(),
            };
        }
        Lookup::NotFound
    }

    /// `GET url` → the `commit.id` of the 200 JSON body.
    async fn get_commit_id(&self, url: &str) -> Lookup {
        match self.get_text(url).await {
            Ok(Some(text)) => parse_commit_id(&text),
            Ok(None) => Lookup::NotFound,
            Err(e) => e,
        }
    }

    /// Authenticated `GET url`. 404 → `Ok(None)` (the internal not-found
    /// sentinel); other non-2xx → `Err` with the plan's error mapping;
    /// 2xx → the body text.
    async fn get_text(&self, url: &str) -> Result<Option<String>, Lookup> {
        let mut req = self.http.get(url);
        if let Some(token) = &self.token {
            let header_name = if self.job_token {
                "JOB-TOKEN"
            } else {
                "PRIVATE-TOKEN"
            };
            req = req.header(header_name, token.as_str());
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                return Err(Lookup::Error(GitLabError::Unresolved(format!(
                    "gitlab api: network error: {e}"
                ))));
            }
        };
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let msg = match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => format!(
                    "gitlab api: {} unauthorized (check GITLAB_TOKEN/CI_JOB_TOKEN)",
                    status.as_u16()
                ),
                StatusCode::TOO_MANY_REQUESTS => {
                    "gitlab api: rate limited (429), retry later".to_string()
                }
                _ => format!("gitlab api: {status}"),
            };
            return Err(Lookup::Error(GitLabError::Unresolved(msg)));
        }
        let text = resp.text().await.map_err(|e| {
            Lookup::Error(GitLabError::Unresolved(format!(
                "gitlab api: network error: {e}"
            )))
        })?;
        Ok(Some(text))
    }

    /// The base URL without a trailing slash (callers append paths).
    fn base(&self) -> &str {
        self.base_url.trim_end_matches('/')
    }
}

/// The GitLab clients of one run, keyed by host; `primary` is the
/// `--gitlab-host` instance (includes resolve there). Crate-private: plugins
/// reach it only through `Ctx::gitlab_resolve` (D38).
pub(crate) struct GitLabHosts {
    primary: String,
    clients: BTreeMap<String, GitLabClient>,
}

impl GitLabHosts {
    /// `clients` keyed by host; `primary` must be one of them.
    pub(crate) fn new(primary: &str, clients: BTreeMap<String, GitLabClient>) -> Self {
        debug_assert!(clients.contains_key(primary), "primary has a client");
        Self {
            primary: primary.to_string(),
            clients,
        }
    }

    /// One host, which is also the primary.
    pub(crate) fn single(host: &str, client: GitLabClient) -> Self {
        Self {
            primary: host.to_string(),
            clients: BTreeMap::from([(host.to_string(), client)]),
        }
    }

    /// The primary host's name (`None` callers of `Ctx::gitlab_resolve`).
    pub(crate) fn primary_host(&self) -> &str {
        &self.primary
    }

    pub(crate) fn get(&self, host: &str) -> Option<&GitLabClient> {
        self.clients.get(host)
    }
}

/// RFC 3986 percent-encoding: unreserved characters (`A-Z a-z 0-9 - _ . ~`)
/// pass through; everything else becomes `%XX` (uppercase hex).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_table() {
        assert_eq!(url_encode("a/b c"), "a%2Fb%20c");
        assert_eq!(url_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(url_encode("v1.0.0"), "v1.0.0");
    }

    /// A 40-hex lowercase SHA is returned as-is with ZERO HTTP requests:
    /// the mock server answers unmatched requests with 404, so any request
    /// would surface as an error.
    #[tokio::test]
    async fn sha_short_circuit() {
        let server = wiremock::MockServer::start().await;
        let c = GitLabClient::new(server.uri(), None, false);
        let sha = "e3262fdd0914fa823210cdb79a8c421e2cef79d8";
        assert_eq!(c.resolve("g/p", sha).await.unwrap(), sha);
        assert!(
            server
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty(),
            "SHA resolution must not hit the network"
        );
    }
}
