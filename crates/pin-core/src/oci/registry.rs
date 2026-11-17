//! Registry v2 client: resolve an image reference to its immutable manifest
//! digest on any registry host (M4a Task 2: the M1 Docker-Hub-only gate is
//! gone; without a `base_urls` override the base is the Docker Hub default
//! for `docker.io`, else `https://{host}`).
//!
//! Flow: anonymous `HEAD /v2/{repo}/manifests/{tag}`; on 401 parse
//! `WWW-Authenticate`, fetch a bearer token with the ref's host's
//! credentials (per-host Basic or Bearer, M4a), retry with
//! `Authorization: Bearer`. The digest comes from the
//! `Docker-Content-Digest` header, falling back to `GET` + sha256 of the body.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::oci::image_ref::ImageRef;
use crate::ver::age::parse_rfc3339_unix;

const DEFAULT_DOCKER_HUB_BASE: &str = "https://registry-1.docker.io";
/// Default Docker Hub API base (M4b-1 `tag_timestamps`); the tags URL is
/// `{base}/v2/repositories/<repo>/tags` — the base itself must NOT carry
/// a `/v2` segment.
const DEFAULT_HUB_API_BASE: &str = "https://hub.docker.com";

/// Docker Hub tags API URL for one page (M4b-1).
fn hub_tags_url(base: &str, repo: &str) -> String {
    format!("{base}/v2/repositories/{repo}/tags?page_size=100")
}
const MANIFEST_ACCEPT: &str = "application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.index.v1+json";

/// Errors from registry lookups. Plain enum + manual `Display`; no thiserror.
/// `Clone` so the per-run memo can cache failed lookups (§6.7, review L4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegError {
    /// The reference could not be resolved (404, bad token response, ...).
    Unresolved(String),
    /// Transport or HTTP-level failure.
    Network(String),
}

impl fmt::Display for RegError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegError::Unresolved(msg) => write!(f, "unresolved: {msg}"),
            RegError::Network(msg) => write!(f, "network error: {msg}"),
        }
    }
}

/// Per-host registry credentials (M4a). `Debug` redacts the secret
/// (T-D29).
#[derive(Clone, PartialEq)]
pub enum HostCreds {
    /// HTTP Basic (registry token endpoint), e.g. Docker Hub.
    Basic { user: String, password: String },
    /// Pre-issued token sent as `Authorization: Bearer`, e.g. GitHub
    /// Packages.
    Bearer { token: String },
}

impl fmt::Debug for HostCreds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secret = crate::secret::Redacted;
        match self {
            HostCreds::Basic { user, .. } => f
                .debug_struct("Basic")
                .field("user", user)
                .field("password", &secret)
                .finish(),
            HostCreds::Bearer { .. } => f.debug_struct("Bearer").field("token", &secret).finish(),
        }
    }
}

/// Registry credentials keyed by registry host (e.g. `"docker.io"`,
/// `"ghcr.io"`). M4a: per-host Basic or Bearer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Creds {
    hosts: HashMap<String, HostCreds>,
}

impl Creds {
    /// No credentials: every request is anonymous. Only test and
    /// `test-support` code build a `Creds` directly; production reads it
    /// through `ResolverEnv`.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    pub fn none() -> Self {
        Self::default()
    }

    /// Environment (R2) + `~/.docker/config.json` per the constraint-4
    /// precedence table. Env vars win per host: `DOCKERHUB_USER`+
    /// `DOCKERHUB_TOKEN` -> `docker.io` Basic, `GITHUB_TOKEN` -> `ghcr.io`
    /// Bearer. A missing or malformed config.json yields no entry for that
    /// host, never fatal.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    pub fn load_from(home: Option<&Path>) -> Self {
        Self::load_from_env(home, |var| std::env::var(var).ok())
    }

    /// Same as [`Creds::load_from`] but env vars are read through `env`
    /// (`Ctx` passes the env map, PR-M2; unit tests inject).
    pub(crate) fn load_from_env<F>(home: Option<&Path>, env: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut hosts: HashMap<String, HostCreds> = HashMap::new();
        // config.json first (lower precedence); env vars override per host.
        if let Some(home) = home
            && let Some(from_config) = Self::hosts_from_config(home)
        {
            hosts.extend(from_config);
        }
        if let (Some(user), Some(password)) = (
            env("DOCKERHUB_USER").filter(|s| !s.is_empty()),
            env("DOCKERHUB_TOKEN").filter(|s| !s.is_empty()),
        ) {
            hosts.insert("docker.io".to_string(), HostCreds::Basic { user, password });
        }
        if let Some(token) = env("GITHUB_TOKEN").filter(|s| !s.is_empty()) {
            hosts.insert("ghcr.io".to_string(), HostCreds::Bearer { token });
        }
        Self { hosts }
    }

    /// Credentials for `host`, if any.
    pub fn for_host(&self, host: &str) -> Option<&HostCreds> {
        self.hosts.get(host)
    }

    /// Test hook: add/replace the credentials for one host.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    pub fn with_host(mut self, host: &str, creds: HostCreds) -> Self {
        self.hosts.insert(host.to_string(), creds);
        self
    }

    /// Credentials from `home/.docker/config.json` per the constraint-4
    /// key table. Missing or malformed file -> `None` (never fatal).
    fn hosts_from_config(home: &Path) -> Option<HashMap<String, HostCreds>> {
        let raw = std::fs::read_to_string(home.join(".docker").join("config.json")).ok()?;
        let config: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let auths = config.get("auths")?.as_object()?;
        // Distinct hosts present in config.json (reverse key -> host map).
        let mut host_list: Vec<&str> = Vec::new();
        for key in auths.keys() {
            let host = host_from_config_key(key);
            if !host_list.contains(&host) {
                host_list.push(host);
            }
        }
        let mut hosts: HashMap<String, HostCreds> = HashMap::new();
        for host in host_list {
            if let Some(creds) = creds_from_auths(auths, host) {
                hosts.insert(host.to_string(), creds);
            }
        }
        Some(hosts)
    }
}

/// Reverse config.json key -> host map (constraint 4).
fn host_from_config_key(key: &str) -> &str {
    match key {
        "docker.io"
        | "index.docker.io/v1/"
        | "https://docker.io"
        | "https://index.docker.io/v1/" => "docker.io",
        "ghcr.io" | "https://ghcr.io" => "ghcr.io",
        k => k.strip_prefix("https://").unwrap_or(k),
    }
}

/// Config keys to look up for `host`, in precedence order (constraint 4;
/// docker.io keeps the M1 key list and order).
fn config_keys_for_host(host: &str) -> Vec<String> {
    if host == "docker.io" {
        return vec![
            "https://index.docker.io/v1/".to_string(),
            "https://docker.io".to_string(),
            "index.docker.io/v1/".to_string(),
            "docker.io".to_string(),
        ];
    }
    vec![host.to_string(), format!("https://{host}")]
}

/// For host `host`, the first key present in `auths` decides (M1
/// semantics): its entry is converted, and a malformed entry yields `None`
/// for the host (no fall-through to later keys).
fn creds_from_auths(
    auths: &serde_json::Map<String, serde_json::Value>,
    host: &str,
) -> Option<HostCreds> {
    for key in config_keys_for_host(host) {
        let Some(entry) = auths.get(&key) else {
            continue;
        };
        return entry_to_creds(entry);
    }
    None
}

/// Config entry -> credentials: `username`+`password` -> Basic, else
/// `auth` (base64 `user:password`) -> Basic, else `identitytoken` ->
/// Bearer, else `None`.
fn entry_to_creds(entry: &serde_json::Value) -> Option<HostCreds> {
    if let (Some(user), Some(pass)) = (
        entry.get("username").and_then(serde_json::Value::as_str),
        entry.get("password").and_then(serde_json::Value::as_str),
    ) {
        return Some(HostCreds::Basic {
            user: user.to_string(),
            password: pass.to_string(),
        });
    }
    // What `docker login` writes: `auth` = base64("user:password").
    if let Some((user, password)) = entry
        .get("auth")
        .and_then(serde_json::Value::as_str)
        .and_then(decode_basic_auth)
    {
        return Some(HostCreds::Basic { user, password });
    }
    if let Some(token) = entry
        .get("identitytoken")
        .and_then(serde_json::Value::as_str)
    {
        return Some(HostCreds::Bearer {
            token: token.to_string(),
        });
    }
    None
}

/// `base64("user:password")` (standard alphabet, padding optional) ->
/// `(user, password)`, split at the first `:`; anything else -> `None`.
fn decode_basic_auth(b64: &str) -> Option<(String, String)> {
    let mut bits: u32 = 0;
    let mut n = 0;
    let mut out = Vec::with_capacity(b64.len() * 3 / 4);
    for c in b64.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    let text = String::from_utf8(out).ok()?;
    let (user, password) = text.split_once(':')?;
    (!user.is_empty()).then(|| (user.to_string(), password.to_string()))
}

/// Token cache key: `[host, realm, service, scope]`.
type TokenKey = [String; 4];

impl fmt::Debug for RegistryClient {
    /// `creds` print redacted; cached bearer tokens are never printed
    /// (M5d review-1 L5).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegistryClient")
            .field("creds", &self.creds)
            .field("base_urls", &self.base_urls)
            .field("hub_base", &self.hub_base)
            .field("tokens", &crate::secret::Redacted)
            .finish_non_exhaustive()
    }
}

/// Client for the registry HTTP v2 API (M4a: per-host credentials; every
/// host resolves — `base_urls` override, Docker Hub default, else
/// `https://{host}`).
///
/// The token cache is a `tokio::sync::Mutex` so `digest` is concurrency-safe
/// (M1's orchestrator calls it from concurrent tasks). The cache sits behind
/// `Arc`, so clones share it (M5c, P11). Lookups are memoised per run in
/// `Ctx`, not here (D19). `Debug` prints no secret (P-redact).
#[derive(Clone)]
pub struct RegistryClient {
    http: reqwest::Client,
    creds: Creds,
    base_urls: HashMap<String, String>,
    /// Test hook: override the Docker Hub API base (M4b-1);
    /// `None` → `DEFAULT_HUB_API_BASE` (tags URL `{base}/v2/repositories/…`).
    hub_base: Option<String>,
    /// Bearer tokens keyed by `(host, realm, service, scope)`: a token is
    /// only ever replayed to the host that challenged for it (§7.3,
    /// review H1).
    tokens: Arc<Mutex<HashMap<TokenKey, String>>>,
}

impl RegistryClient {
    /// `base_urls` maps a registry host key (e.g. `"docker.io"`) to a base URL;
    /// a missing `"docker.io"` entry falls back to `https://registry-1.docker.io`.
    /// Panics if the HTTP client cannot be built (test convenience; see
    /// [`RegistryClient::try_new`]).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn new(creds: Creds, base_urls: HashMap<String, String>) -> Self {
        Self::try_new(creds, base_urls).expect("reqwest client builds")
    }

    /// Like [`RegistryClient::new`], but an HTTP client construction error
    /// is returned as `"http client: {e}"` (review L5).
    pub fn try_new(creds: Creds, base_urls: HashMap<String, String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self {
            http,
            creds,
            base_urls,
            hub_base: None,
            tokens: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Test hook: point the Docker Hub API (M4b-1 `tag_timestamps`) at a
    /// mock server.
    pub fn with_hub_base(mut self, url: &str) -> Self {
        self.hub_base = Some(url.to_string());
        self
    }

    /// Registry base URL for `r`'s host: the `base_urls` override, then the
    /// Docker Hub default for `"docker.io"`, then `https://{host}` (M4a
    /// Task 2: every host resolves; the M1 "no base URL" error is gone).
    fn base_for(&self, r: &ImageRef) -> String {
        let host = r.host_str();
        if let Some(base) = self.base_urls.get(host) {
            return base.clone();
        }
        if host == "docker.io" {
            DEFAULT_DOCKER_HUB_BASE.to_string()
        } else {
            format!("https://{host}")
        }
    }

    /// Resolve `r` to `"sha256:<64 hex>"`.
    pub async fn digest(&self, r: &ImageRef) -> Result<String, RegError> {
        let base = self.base_for(r);
        let path = format!("/v2/{}/manifests/{}", r.api_repo(), r.effective_tag());
        let not_found =
            || RegError::Unresolved(format!("{}:{} not found", r.api_repo(), r.effective_tag()));

        let first = self
            .http
            .head(format!("{base}{path}"))
            .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
            .send()
            .await
            .map_err(|e| RegError::Network(e.to_string()))?;

        match first.status() {
            s if s.is_success() => self.digest_from_head(r, first, &base, &path).await,
            StatusCode::UNAUTHORIZED => {
                let Some(www) = first
                    .headers()
                    .get(reqwest::header::WWW_AUTHENTICATE)
                    .and_then(|v| v.to_str().ok())
                else {
                    return Err(RegError::Unresolved(format!(
                        "401 without WWW-Authenticate at {path}"
                    )));
                };
                let retry = self
                    .challenge_retry(
                        reqwest::Method::HEAD,
                        &format!("{base}{path}"),
                        Some(MANIFEST_ACCEPT),
                        r.host_str(),
                        www,
                        &path,
                    )
                    .await?;
                match retry.status() {
                    s if s.is_success() => self.digest_from_head(r, retry, &base, &path).await,
                    StatusCode::UNAUTHORIZED => {
                        Err(RegError::Unresolved("authentication failed".to_string()))
                    }
                    StatusCode::NOT_FOUND => Err(not_found()),
                    s => Err(RegError::Network(format!("{s} at {path}"))),
                }
            }
            StatusCode::NOT_FOUND => Err(not_found()),
            s => Err(RegError::Network(format!("{s} at {path}"))),
        }
    }

    /// Digest from a successful HEAD: `Docker-Content-Digest` header when
    /// present and starting with `sha256:`, else `GET` + sha256 of the body.
    async fn digest_from_head(
        &self,
        r: &ImageRef,
        head: reqwest::Response,
        base: &str,
        path: &str,
    ) -> Result<String, RegError> {
        if let Some(digest) = head
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .filter(|d| d.starts_with("sha256:"))
        {
            return Ok(digest.to_string());
        }
        let resp = self
            .get_authed(
                &format!("{base}{path}"),
                Some(MANIFEST_ACCEPT),
                r.host_str(),
            )
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return if status == StatusCode::NOT_FOUND {
                Err(RegError::Unresolved(format!(
                    "{}:{} not found",
                    r.api_repo(),
                    r.effective_tag()
                )))
            } else {
                Err(RegError::Network(format!("{status} at {path}")))
            };
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| RegError::Network(e.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(&body);
        let sum = hasher.finalize();
        let hex: String = sum.iter().map(|b| format!("{b:02x}")).collect();
        Ok(format!("sha256:{hex}"))
    }

    /// Anonymous `GET url` (with `Accept: {accept}` when `accept` is
    /// `Some`); on 401 dispatch the `WWW-Authenticate` challenge per host
    /// (see [`challenge_retry`]) and retry once. Returns the final
    /// response; the caller handles its status.
    async fn get_authed(
        &self,
        url: &str,
        accept: Option<&str>,
        host: &str,
    ) -> Result<reqwest::Response, RegError> {
        let mut req = self.http.get(url);
        if let Some(accept) = accept {
            req = req.header(reqwest::header::ACCEPT, accept);
        }
        let first = req
            .send()
            .await
            .map_err(|e| RegError::Network(e.to_string()))?;
        if first.status() != StatusCode::UNAUTHORIZED {
            return Ok(first);
        }
        let Some(www) = first
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
        else {
            return Err(RegError::Unresolved(format!(
                "401 without WWW-Authenticate at {url}"
            )));
        };
        self.challenge_retry(reqwest::Method::GET, url, accept, host, www, url)
            .await
    }

    /// Dispatch a 401 challenge (`www` header) for a request to `url`,
    /// returning the authenticated retry response:
    /// - `Bearer realm=…` → the per-host token (cached per host and
    ///   challenge; the token request itself carries the per-host
    ///   credential, M4a T1) and
    ///   `Authorization: Bearer` on the retry. The scope is trusted as
    ///   issued by the registry (ECR public uses `scope=aws`, not the
    ///   Docker-Hub `repository:…:pull` grammar).
    /// - `Basic realm=…` → the manifest is retried directly with the
    ///   per-host Basic login (no token round-trip, JFrog style); a
    ///   missing or Bearer-only per-host entry fails loud with a
    ///   `docker login` hint (R1).
    /// - any other scheme → the M1 unparseable-challenge error.
    async fn challenge_retry(
        &self,
        method: reqwest::Method,
        url: &str,
        accept: Option<&str>,
        host: &str,
        www: &str,
        label: &str,
    ) -> Result<reqwest::Response, RegError> {
        if let Some((realm, service, scope)) = parse_www_authenticate(www) {
            let token = self.cached_token(&scope, &realm, &service, host).await?;
            return self
                .retry_request(method, url, accept, format!("Bearer {token}"))
                .await;
        }
        if www
            .split_whitespace()
            .next()
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("basic"))
        {
            return match self.creds.for_host(host) {
                Some(HostCreds::Basic { user, password }) => {
                    let mut req = self.http.request(method, url);
                    if let Some(a) = accept {
                        req = req.header(reqwest::header::ACCEPT, a);
                    }
                    req = req.basic_auth(user, Some(password));
                    req.send()
                        .await
                        .map_err(|e| RegError::Network(e.to_string()))
                }
                _ => Err(RegError::Unresolved(format!(
                    "{host}: registry requires basic login — docker login {host}"
                ))),
            };
        }
        Err(RegError::Unresolved(format!(
            "unparseable WWW-Authenticate header at {label}: {www}"
        )))
    }

    /// Send `method url` with `Authorization: {authorization}` (plus
    /// `Accept: {accept}` when `Some`).
    async fn retry_request(
        &self,
        method: reqwest::Method,
        url: &str,
        accept: Option<&str>,
        authorization: String,
    ) -> Result<reqwest::Response, RegError> {
        let mut req = self
            .http
            .request(method, url)
            .header(reqwest::header::AUTHORIZATION, authorization);
        if let Some(a) = accept {
            req = req.header(reqwest::header::ACCEPT, a);
        }
        req.send()
            .await
            .map_err(|e| RegError::Network(e.to_string()))
    }

    /// Push timestamps (unix seconds) for every tag of a Docker Hub ref,
    /// from the Docker Hub API (M4b-1 release-age guard). Non-Hub refs
    /// short-circuit to `Ok(empty)` without any HTTP. Pages with
    /// `?page_size=100` and follows absolute `next` URLs until exhausted.
    /// Per tag, `tag_last_pushed` is preferred over `last_updated`; a tag
    /// with neither gets no entry. A 404 (unknown repo) yields `Ok(empty)`;
    /// any other failure (network, 429, malformed page) is a `RegError`
    /// with the `hub api:` prefix. No retry on 429 (M4a precedent).
    pub async fn tag_timestamps(&self, r: &ImageRef) -> Result<BTreeMap<String, i64>, RegError> {
        if !r.is_docker_hub() {
            return Ok(BTreeMap::new());
        }
        let base = self
            .hub_base
            .clone()
            .unwrap_or_else(|| DEFAULT_HUB_API_BASE.to_string());
        let mut url = hub_tags_url(&base, &r.api_repo());
        let mut out: BTreeMap<String, i64> = BTreeMap::new();
        loop {
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| RegError::Network(e.to_string()))?;
            let status = resp.status();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Ok(BTreeMap::new());
            }
            if !status.is_success() {
                return Err(RegError::Unresolved(format!("hub api: {status} at {url}")));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| RegError::Network(e.to_string()))?;
            let page: serde_json::Value = serde_json::from_str(&text)
                .map_err(|_| RegError::Unresolved(format!("hub api: malformed page at {url}")))?;
            let results = page
                .get("results")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| RegError::Unresolved(format!("hub api: malformed page at {url}")))?;
            for t in results {
                let Some(name) = t.get("name").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let ts = t
                    .get("tag_last_pushed")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| t.get("last_updated").and_then(serde_json::Value::as_str))
                    .and_then(parse_rfc3339_unix);
                if let Some(ts) = ts {
                    out.insert(name.to_string(), ts);
                }
            }
            let Some(next) = page.get("next").and_then(serde_json::Value::as_str) else {
                break;
            };
            if next.is_empty() {
                break;
            }
            url = next.to_string();
        }
        Ok(out)
    }

    /// All tags of the repo, following `Link: <url>; rel="next"` until
    /// exhausted. First request carries `?n=1000`. Anonymous probe first;
    /// 401 → WWW-Authenticate → bearer token (scope `repository:{repo}:pull`)
    /// → retry. 404 → `Unresolved("{repo} not found")`. Not cached here:
    /// the per-run memo (`Ctx::oci_tags`, keyed `(host, api_repo)`) is the
    /// one cache (D19, review M1).
    pub async fn list_tags(&self, r: &ImageRef) -> Result<Vec<String>, RegError> {
        let repo = r.api_repo();
        let base = self.base_for(r);
        let mut tags: Vec<String> = Vec::new();
        let mut url = format!("{base}/v2/{repo}/tags/list?n=1000");
        loop {
            let resp = self.get_authed(&url, None, r.host_str()).await?;
            match resp.status() {
                StatusCode::NOT_FOUND => {
                    return Err(RegError::Unresolved(format!("{repo} not found")));
                }
                StatusCode::UNAUTHORIZED => {
                    return Err(RegError::Unresolved("authentication failed".to_string()));
                }
                s if s.is_success() => {}
                s => return Err(RegError::Network(format!("{s} at {url}"))),
            }
            // `text()` consumes the response; take the `Link` header first.
            let next_link = resp.headers().get(reqwest::header::LINK).cloned();
            // reqwest's `json()` method needs its `json` feature (not
            // enabled); parse the body text with serde_json instead. A
            // missing/`null` `tags` array is an empty list.
            let text = resp
                .text()
                .await
                .map_err(|e| RegError::Network(e.to_string()))?;
            let body: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| RegError::Network(format!("invalid tag list JSON at {url}: {e}")))?;
            if let Some(arr) = body.get("tags").and_then(serde_json::Value::as_array) {
                for tag in arr {
                    if let Some(t) = tag.as_str() {
                        tags.push(t.to_string());
                    }
                }
            }
            match parse_link_next(next_link.as_ref()) {
                Some(next) => url = resolve_link_url(&next, &base),
                None => break,
            }
        }
        Ok(tags)
    }

    /// Bearer token for `host`'s challenge `(realm, service, scope)`, from
    /// the cache or the token endpoint (authed per `host`).
    async fn cached_token(
        &self,
        scope: &str,
        realm: &str,
        service: &str,
        host: &str,
    ) -> Result<String, RegError> {
        let key: TokenKey = [host, realm, service, scope].map(str::to_string);
        {
            let cache = self.tokens.lock().await;
            if let Some(token) = cache.get(&key) {
                return Ok(token.clone());
            }
        }
        let token = self.fetch_token(realm, service, scope, host).await?;
        self.tokens.lock().await.insert(key, token.clone());
        Ok(token)
    }

    /// `GET {realm}?service={service}&scope={scope}` with the host's
    /// credentials: `Basic` -> HTTP Basic auth, `Bearer` ->
    /// `Authorization: Bearer {token}`, none -> no `Authorization` header.
    /// Expects JSON `{"token": ...}` or `{"access_token": ...}`.
    async fn fetch_token(
        &self,
        realm: &str,
        service: &str,
        scope: &str,
        host: &str,
    ) -> Result<String, RegError> {
        let mut req = self
            .http
            .get(realm)
            .query(&[("service", service), ("scope", scope)]);
        match self.creds.for_host(host) {
            Some(HostCreds::Basic { user, password }) => {
                req = req.basic_auth(user, Some(password));
            }
            Some(HostCreds::Bearer { token }) => {
                req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"));
            }
            None => {}
        }
        let resp = req
            .send()
            .await
            .map_err(|e| RegError::Network(e.to_string()))?;
        let status = resp.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            let hint = if host == "ghcr.io" {
                "set GITHUB_TOKEN (PAT with read:packages) or docker login ghcr.io".to_string()
            } else {
                format!("docker login {host}")
            };
            return Err(RegError::Unresolved(format!(
                "{host}: token denied — {hint}"
            )));
        }
        if !status.is_success() {
            return Err(RegError::Unresolved(format!(
                "token endpoint returned {status} at {realm}"
            )));
        }
        // reqwest's `json()` method needs its `json` feature (not enabled);
        // parse the body text with serde_json instead.
        let text = resp
            .text()
            .await
            .map_err(|e| RegError::Unresolved(format!("token response unreadable: {e}")))?;
        let body: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| RegError::Unresolved(format!("token response is not JSON: {e}")))?;
        let token = body
            .get("token")
            .or_else(|| body.get("access_token"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                RegError::Unresolved(format!("token response has no token field: {realm}"))
            })?;
        Ok(token)
    }
}

/// Parse `Bearer realm="R",service="S",scope="SC"`: split on commas outside
/// quotes, keys `realm`/`service`/`scope`, values quoted strings.
fn parse_www_authenticate<'a>(header: &'a str) -> Option<(String, String, String)> {
    let rest = header.strip_prefix("Bearer")?.trim_start();
    let mut kv: HashMap<&'a str, &'a str> = HashMap::new();
    let mut push = |part: &'a str| -> Option<()> {
        let part = part.trim();
        if !part.is_empty() {
            let (key, value) = part.split_once('=')?;
            let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
            kv.insert(key.trim(), value);
        }
        Some(())
    };
    let mut in_quotes = false;
    let mut start = 0;
    for (i, c) in rest.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                push(&rest[start..i])?;
                start = i + 1;
            }
            _ => {}
        }
    }
    push(&rest[start..])?;
    Some((
        kv.remove("realm")?.to_string(),
        kv.remove("service")?.to_string(),
        kv.remove("scope")?.to_string(),
    ))
}

/// Parse a `Link` header, returning the URL whose `rel` is `"next"`.
/// Entries are comma-separated, each of the form `<url>; params`.
/// Resolve a `Link` URL against the registry base. Registries differ: Docker
/// Hub sends PATH-RELATIVE links (`/v2/...` — required for tag-list
/// pagination once a repo passes the `n=1000` first page), others send
/// absolute URLs. An unresolved relative URL would make reqwest fail to
/// build the follow-up request ("builder error").
fn resolve_link_url(link: &str, base: &str) -> String {
    if link.starts_with("http://") || link.starts_with("https://") {
        link.to_string()
    } else if let Some(rest) = link.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        format!("{base}/{}", link.trim_start_matches('/'))
    }
}

fn parse_link_next(header: Option<&reqwest::header::HeaderValue>) -> Option<String> {
    let raw = header?.to_str().ok()?;
    for part in raw.split(',') {
        let part = part.trim();
        let Some((url_part, params)) = part.split_once(';') else {
            continue;
        };
        let url = url_part.trim();
        let Some(url) = url.strip_prefix('<').and_then(|u| u.strip_suffix('>')) else {
            continue;
        };
        if params.contains(r#"rel="next""#) {
            return Some(url.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reg_error_is_clone_and_eq() {
        let e = RegError::Network("x".into());
        assert_eq!(e.clone(), e);
        assert_ne!(e, RegError::Unresolved("x".into()));
    }

    #[test]
    fn base_for_precedence() {
        // A base_urls override wins for any host.
        let c = RegistryClient::new(
            Creds::none(),
            HashMap::from([("ghcr.io".to_string(), "http://mock-ghcr".to_string())]),
        );
        let r = ImageRef::parse("ghcr.io/org/img:1.0").unwrap();
        assert_eq!(c.base_for(&r), "http://mock-ghcr");
        // No overrides: docker.io keeps its default; every other host is
        // https://{host} (M4a Task 2).
        let c = RegistryClient::new(Creds::none(), HashMap::new());
        assert_eq!(
            c.base_for(&ImageRef::parse("alpine:3.20").unwrap()),
            DEFAULT_DOCKER_HUB_BASE
        );
        assert_eq!(
            c.base_for(&ImageRef::parse("docker.io/alpine:3.20").unwrap()),
            DEFAULT_DOCKER_HUB_BASE
        );
        assert_eq!(
            c.base_for(&ImageRef::parse("ghcr.io/org/img:1.0").unwrap()),
            "https://ghcr.io"
        );
        assert_eq!(
            c.base_for(&ImageRef::parse("reg.example.com:5000/a/b:1").unwrap()),
            "https://reg.example.com:5000"
        );
    }

    #[test]
    fn www_authenticate_parses_all_keys() {
        let (realm, service, scope) = parse_www_authenticate(
            r#"Bearer realm="https://auth.example/token",service="registry.example",scope="repository:library/alpine:pull""#,
        )
        .unwrap();
        assert_eq!(realm, "https://auth.example/token");
        assert_eq!(service, "registry.example");
        assert_eq!(scope, "repository:library/alpine:pull");
    }

    #[test]
    fn www_authenticate_keeps_commas_inside_quotes() {
        let (_, _, scope) = parse_www_authenticate(
            r#"Bearer realm="https://auth.example/token",service="s",scope="a,b,c""#,
        )
        .unwrap();
        assert_eq!(scope, "a,b,c");
    }

    #[test]
    fn www_authenticate_rejects_bad_headers() {
        assert!(parse_www_authenticate(r#"Basic realm="x""#).is_none());
        assert!(parse_www_authenticate(r#"Bearer realm="x",service="s""#).is_none());
        assert!(parse_www_authenticate(r#"Bearer realm=x,service="s",scope="t""#).is_none());
    }

    /// P-redact (M5d review-1 L5): neither the credentials nor a cached
    /// bearer token reach `Debug` output.
    #[test]
    fn debug_hides_cached_tokens() {
        let c = RegistryClient::new(
            Creds::none().with_host(
                "ghcr.io",
                HostCreds::Bearer {
                    token: "pat-s3cr3t".into(),
                },
            ),
            HashMap::new(),
        );
        c.tokens.try_lock().unwrap().insert(
            ["h", "r", "s", "sc"].map(str::to_string),
            "tok-s3cr3t".into(),
        );
        let printed = format!("{c:?}");
        assert!(!printed.contains("s3cr3t"), "secret leaked: {printed}");
        assert!(printed.contains("<redacted>"), "{printed}");
    }

    #[test]
    fn creds_none_is_default() {
        assert!(Creds::none().for_host("docker.io").is_none());
        assert_eq!(Creds::none(), Creds::default());
    }

    fn write_docker_config(home: &std::path::Path, json: &str) {
        std::fs::create_dir_all(home.join(".docker")).unwrap();
        std::fs::write(home.join(".docker").join("config.json"), json).unwrap();
    }

    /// A fresh per-test home dir under `std::env::temp_dir()`.
    fn fresh_home(tag: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("pin-ci-cfg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn creds_env_dockerhub_basic() {
        let env = |var: &str| match var {
            "DOCKERHUB_USER" => Some("hubuser".to_string()),
            "DOCKERHUB_TOKEN" => Some("hubtoken".to_string()),
            _ => None,
        };
        let c = Creds::load_from_env(None, env);
        assert_eq!(
            c.for_host("docker.io"),
            Some(&HostCreds::Basic {
                user: "hubuser".to_string(),
                password: "hubtoken".to_string(),
            })
        );
        assert_eq!(c.for_host("ghcr.io"), None);
    }

    #[test]
    fn creds_env_ghcr_bearer() {
        let env = |var: &str| match var {
            "GITHUB_TOKEN" => Some("ghpat".to_string()),
            _ => None,
        };
        let c = Creds::load_from_env(None, env);
        assert_eq!(
            c.for_host("ghcr.io"),
            Some(&HostCreds::Bearer {
                token: "ghpat".to_string()
            })
        );
        assert_eq!(c.for_host("docker.io"), None);
    }

    #[test]
    fn creds_env_beats_config() {
        let home = fresh_home("env-beats-config");
        write_docker_config(
            &home,
            r#"{"auths":{"docker.io":{"username":"cu","password":"cp"},"ghcr.io":{"identitytoken":"ctok"}}}"#,
        );
        let env = |var: &str| match var {
            "DOCKERHUB_USER" => Some("eu".to_string()),
            "DOCKERHUB_TOKEN" => Some("ep".to_string()),
            "GITHUB_TOKEN" => Some("etok".to_string()),
            _ => None,
        };
        let c = Creds::load_from_env(Some(&home), env);
        assert_eq!(
            c.for_host("docker.io"),
            Some(&HostCreds::Basic {
                user: "eu".to_string(),
                password: "ep".to_string(),
            })
        );
        assert_eq!(
            c.for_host("ghcr.io"),
            Some(&HostCreds::Bearer {
                token: "etok".to_string()
            })
        );
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn creds_config_dockerhub_keys() {
        // Each of the four M1 key forms resolves to docker.io Basic.
        for key in [
            "https://index.docker.io/v1/",
            "https://docker.io",
            "index.docker.io/v1/",
            "docker.io",
        ] {
            let home = fresh_home("dockerhub-keys");
            write_docker_config(
                &home,
                &format!(r#"{{"auths":{{"{key}":{{"username":"u","password":"p"}}}}}}"#),
            );
            let c = Creds::load_from_env(Some(&home), |_| None);
            assert_eq!(
                c.for_host("docker.io"),
                Some(&HostCreds::Basic {
                    user: "u".to_string(),
                    password: "p".to_string(),
                }),
                "config key {key}"
            );
            std::fs::remove_dir_all(&home).ok();
        }
        // Multiple keys present: the earliest key in the M1 list wins.
        let home = fresh_home("dockerhub-key-order");
        write_docker_config(
            &home,
            r#"{"auths":{"docker.io":{"username":"last","password":"p"},"https://index.docker.io/v1/":{"username":"first","password":"p"}}}"#,
        );
        let c = Creds::load_from_env(Some(&home), |_| None);
        assert_eq!(
            c.for_host("docker.io"),
            Some(&HostCreds::Basic {
                user: "first".to_string(),
                password: "p".to_string(),
            })
        );
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn creds_config_identitytoken() {
        let home = fresh_home("identitytoken");
        write_docker_config(
            &home,
            r#"{"auths":{"us-docker.pkg.dev":{"identitytoken":"ya29.xyz"}}}"#,
        );
        let c = Creds::load_from_env(Some(&home), |_| None);
        assert_eq!(
            c.for_host("us-docker.pkg.dev"),
            Some(&HostCreds::Bearer {
                token: "ya29.xyz".to_string()
            })
        );
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn creds_config_auth_field() {
        // What `docker login` writes: `auth` = base64("user:pass"); the
        // password may itself contain ':' (split at the first one only).
        let home = fresh_home("auth-field");
        write_docker_config(
            &home,
            r#"{"auths":{"repo.example.com":{"auth":"dTpwOmFzcw=="},"bad.example.com":{"auth":"%%%"},"nocolon.example.com":{"auth":"dXNlcg=="}}}"#,
        );
        let c = Creds::load_from_env(Some(&home), |_| None);
        assert_eq!(
            c.for_host("repo.example.com"),
            Some(&HostCreds::Basic {
                user: "u".to_string(),
                password: "p:ass".to_string()
            })
        );
        assert_eq!(c.for_host("bad.example.com"), None);
        assert_eq!(c.for_host("nocolon.example.com"), None);
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn creds_config_arbitrary_host() {
        for key in ["artifactory.example.com", "https://artifactory.example.com"] {
            let home = fresh_home("arbitrary-host");
            write_docker_config(
                &home,
                &format!(r#"{{"auths":{{"{key}":{{"username":"au","password":"ap"}}}}}}"#),
            );
            let c = Creds::load_from_env(Some(&home), |_| None);
            assert_eq!(
                c.for_host("artifactory.example.com"),
                Some(&HostCreds::Basic {
                    user: "au".to_string(),
                    password: "ap".to_string(),
                }),
                "config key {key}"
            );
            std::fs::remove_dir_all(&home).ok();
        }
    }

    #[test]
    fn creds_missing_home_malformed() {
        // home=None, no env vars -> none.
        let c = Creds::load_from_env(None, |_| None);
        assert_eq!(c, Creds::none());
        // Malformed config.json -> none, never fatal.
        let home = fresh_home("malformed");
        write_docker_config(&home, "{not json");
        let c = Creds::load_from_env(Some(&home), |_| None);
        assert_eq!(c, Creds::none());
        // Valid JSON but no auths -> none.
        write_docker_config(&home, r#"{"other":true}"#);
        let c = Creds::load_from_env(Some(&home), |_| None);
        assert_eq!(c, Creds::none());
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn resolve_link_url_forms() {
        let base = "https://registry-1.docker.io";
        // Absolute links pass through untouched.
        assert_eq!(
            resolve_link_url("https://r/v2/x/tags/list?last=b", base),
            "https://r/v2/x/tags/list?last=b"
        );
        // Docker Hub form: path-relative.
        assert_eq!(
            resolve_link_url("/v2/x/tags/list?last=b", base),
            "https://registry-1.docker.io/v2/x/tags/list?last=b"
        );
        // Protocol-relative.
        assert_eq!(resolve_link_url("//r/v2/x", base), "https://r/v2/x");
        // Bare relative (defensive).
        assert_eq!(
            resolve_link_url("v2/x/tags/list", base),
            "https://registry-1.docker.io/v2/x/tags/list"
        );
    }

    #[test]
    fn parse_link_next_unit() {
        use reqwest::header::HeaderValue;
        assert_eq!(parse_link_next(None), None);
        let h = HeaderValue::from_str(
            r#"<https://r/v2/x/tags/list?last=a>; rel="prev", <https://r/v2/x/tags/list?last=b>; rel="next""#,
        )
        .unwrap();
        assert_eq!(
            parse_link_next(Some(&h)),
            Some("https://r/v2/x/tags/list?last=b".to_string())
        );
        let junk = HeaderValue::from_str("not <quoted>; rel=next").unwrap();
        assert_eq!(parse_link_next(Some(&junk)), None);
    }

    #[test]
    fn default_hub_api_url_is_single_v2() {
        // The default base must not carry /v2 itself: hub_tags_url appends
        // /v2/repositories/... — a doubled /v2 is a 404, which
        // tag_timestamps maps to Ok(empty) and silently inert-izes the
        // release-age guard (M4b-1 T3 oracle finding).
        assert_eq!(
            hub_tags_url(DEFAULT_HUB_API_BASE, "library/ubuntu"),
            "https://hub.docker.com/v2/repositories/library/ubuntu/tags?page_size=100"
        );
    }
}
