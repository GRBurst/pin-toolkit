//! Docker Hub v2 registry client: resolve an image reference to its immutable
//! manifest digest.
//!
//! Flow: anonymous `HEAD /v2/{repo}/manifests/{tag}`; on 401 parse
//! `WWW-Authenticate`, fetch a bearer token (Basic auth when credentials are
//! present), retry with `Authorization: Bearer`. The digest comes from the
//! `Docker-Content-Digest` header, falling back to `GET` + sha256 of the body.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::image_ref::ImageRef;

const DEFAULT_DOCKER_HUB_BASE: &str = "https://registry-1.docker.io";
const MANIFEST_ACCEPT: &str = "application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.index.v1+json";

/// Errors from registry lookups. Plain enum + manual `Display`; no thiserror.
#[derive(Debug)]
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

/// Registry credentials. M1: Docker Hub only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Creds {
    /// `(user, pass)` for Docker Hub.
    pub dockerhub: Option<(String, String)>,
}

impl Creds {
    /// No credentials (anonymous).
    pub fn none() -> Self {
        Self::default()
    }

    /// Resolve Docker Hub credentials: env user/pass take precedence over
    /// config.json; config.json is read from `home/.docker/config.json` when
    /// `home` is `Some` and the file exists.
    pub fn load_from(env_user: Option<String>, env_pass: Option<String>, home: Option<&Path>) -> Self {
        if let (Some(user), Some(pass)) = (env_user, env_pass)
            && !user.is_empty()
            && !pass.is_empty()
        {
            return Self {
                dockerhub: Some((user, pass)),
            };
        }
        Self {
            dockerhub: dockerhub_from_config(home),
        }
    }
}

fn dockerhub_from_config(home: Option<&Path>) -> Option<(String, String)> {
    let home = home?;
    let raw = std::fs::read_to_string(home.join(".docker").join("config.json")).ok()?;
    let config: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let auths = config.get("auths")?.as_object()?;
    for key in ["https://index.docker.io/v1/", "index.docker.io/v1/", "docker.io"] {
        if let Some(entry) = auths.get(key) {
            let user = entry.get("username")?.as_str()?.to_string();
            let pass = entry.get("password")?.as_str()?.to_string();
            return Some((user, pass));
        }
    }
    None
}

/// Client for the registry HTTP v2 API (M1: Docker Hub).
///
/// The token cache is a `tokio::sync::Mutex` so `digest` is concurrency-safe
/// (M1's orchestrator calls it from concurrent tasks).
#[derive(Debug)]
pub struct RegistryClient {
    http: reqwest::Client,
    creds: Creds,
    base_urls: HashMap<String, String>,
    /// Bearer tokens keyed by the `WWW-Authenticate` scope.
    tokens: Mutex<HashMap<String, String>>,
}

impl RegistryClient {
    /// `base_urls` maps a registry host key (e.g. `"docker.io"`) to a base URL;
    /// a missing `"docker.io"` entry falls back to `https://registry-1.docker.io`.
    pub fn new(creds: Creds, base_urls: HashMap<String, String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client builds");
        Self {
            http,
            creds,
            base_urls,
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve `r` to `"sha256:<64 hex>"`.
    pub async fn digest(&self, r: &ImageRef) -> Result<String, RegError> {
        let base = match self.base_urls.get(r.host_str()).cloned() {
            Some(base) => base,
            None if r.host_str() == "docker.io" => DEFAULT_DOCKER_HUB_BASE.to_string(),
            None => {
                return Err(RegError::Unresolved(format!(
                    "no base URL for registry host {}",
                    r.host_str()
                )))
            }
        };
        let path = format!("/v2/{}/manifests/{}", r.api_repo(), r.effective_tag());
        let not_found = || {
            RegError::Unresolved(format!("{}:{} not found", r.api_repo(), r.effective_tag()))
        };

        let first = self
            .http
            .head(format!("{base}{path}"))
            .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
            .send()
            .await
            .map_err(|e| RegError::Network(e.to_string()))?;

        match first.status() {
            s if s.is_success() => self.digest_from_head(r, first, &base, &path, None).await,
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
                let Some((realm, service, scope)) = parse_www_authenticate(www) else {
                    return Err(RegError::Unresolved(format!(
                        "unparseable WWW-Authenticate header at {path}: {www}"
                    )));
                };
                if scope != format!("repository:{}:pull", r.api_repo()) {
                    return Err(RegError::Unresolved(format!(
                        "unexpected token scope {scope} for {}",
                        r.api_repo()
                    )));
                }
                let token = self.cached_token(&scope, &realm, &service).await?;
                let retry = self
                    .http
                    .head(format!("{base}{path}"))
                    .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
                    .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
                    .send()
                    .await
                    .map_err(|e| RegError::Network(e.to_string()))?;
                match retry.status() {
                    s if s.is_success() => {
                        self.digest_from_head(r, retry, &base, &path, Some(&token)).await
                    }
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
        token: Option<&str>,
    ) -> Result<String, RegError> {
        if let Some(digest) = head
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .filter(|d| d.starts_with("sha256:"))
        {
            return Ok(digest.to_string());
        }
        let mut req = self
            .http
            .get(format!("{base}{path}"))
            .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT);
        if let Some(t) = token {
            req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let resp = req.send().await.map_err(|e| RegError::Network(e.to_string()))?;
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
        let body = resp.bytes().await.map_err(|e| RegError::Network(e.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(&body);
        let sum = hasher.finalize();
        let hex: String = sum.iter().map(|b| format!("{b:02x}")).collect();
        Ok(format!("sha256:{hex}"))
    }

    /// Bearer token for `scope`, from the cache or the token endpoint.
    async fn cached_token(&self, scope: &str, realm: &str, service: &str) -> Result<String, RegError> {
        {
            let cache = self.tokens.lock().await;
            if let Some(token) = cache.get(scope) {
                return Ok(token.clone());
            }
        }
        let token = self.fetch_token(realm, service, scope).await?;
        self.tokens
            .lock()
            .await
            .insert(scope.to_string(), token.clone());
        Ok(token)
    }

    /// `GET {realm}?service={service}&scope={scope}` with HTTP Basic auth when
    /// Docker Hub credentials are present. Expects JSON `{"token": ...}` or
    /// `{"access_token": ...}`.
    async fn fetch_token(&self, realm: &str, service: &str, scope: &str) -> Result<String, RegError> {
        let mut req = self
            .http
            .get(realm)
            .query(&[("service", service), ("scope", scope)]);
        if let Some((user, pass)) = &self.creds.dockerhub {
            req = req.basic_auth(user, Some(pass));
        }
        let resp = req.send().await.map_err(|e| RegError::Network(e.to_string()))?;
        let status = resp.status();
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn creds_none_is_default() {
        assert!(Creds::none().dockerhub.is_none());
        assert_eq!(Creds::none(), Creds::default());
    }
}
