/// A parsed container image reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// `None` | `"docker.io"` | `"ghcr.io"` | `"reg:5000"` ...
    pub host: Option<String>,
    /// As written after the host: `"alpine"`, `"library/alpine"`, `"org/img"`
    pub repo: String,
    /// `None` => `latest`
    pub tag: Option<String>,
    /// `"sha256:<64 hex>"`
    pub digest: Option<String>,
}

impl ImageRef {
    /// Parse a container image reference.
    ///
    /// 1. Reject empty input or any whitespace character.
    /// 2. If the input contains `@`, split at the LAST `@`; the part after must
    ///    start with `sha256:` followed by exactly 64 lowercase hex chars,
    ///    else `Err("invalid digest")`. The part before is parsed as host/repo:tag.
    /// 3. The first `/`-separated component of the remainder is a host iff it
    ///    contains `.` or `:` or equals `localhost`; otherwise host is `None`
    ///    and the component is part of the repo.
    /// 4. Tag: the text after the last `:` that appears after the last `/`,
    ///    else `None`.
    /// 5. Reject an empty repo.
    pub fn parse(input: &str) -> Result<ImageRef, String> {
        if input.is_empty() || input.chars().any(char::is_whitespace) {
            return Err("invalid image reference".to_string());
        }

        // 2. Digest: split at the LAST '@'.
        let (ref_part, digest) = match input.rfind('@') {
            Some(at) => {
                let digest = &input[at + 1..];
                let valid = digest.strip_prefix("sha256:").is_some_and(|hex| {
                    hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                });
                if !valid {
                    return Err("invalid digest".to_string());
                }
                (&input[..at], Some(digest.to_string()))
            }
            None => (input, None),
        };

        // 3. Registry host: first '/'-separated component, iff it looks like a host.
        let (host, rest) = match ref_part.find('/') {
            Some(slash) => {
                let first = &ref_part[..slash];
                if first.contains('.') || first.contains(':') || first == "localhost" {
                    (Some(first.to_string()), &ref_part[slash + 1..])
                } else {
                    (None, ref_part)
                }
            }
            None => (None, ref_part),
        };

        // 4. Tag: text after the last ':' that appears after the last '/'.
        let tag_pos = match rest.rfind(':') {
            Some(colon) if rest.rfind('/').map_or(true, |slash| colon > slash) => Some(colon),
            _ => None,
        };
        let (repo, tag) = match tag_pos {
            Some(colon) => (
                rest[..colon].to_string(),
                Some(rest[colon + 1..].to_string()),
            ),
            None => (rest.to_string(), None),
        };

        // 5. Reject an empty repo.
        if repo.is_empty() {
            return Err("empty repository".to_string());
        }

        Ok(ImageRef {
            host,
            repo,
            tag,
            digest,
        })
    }

    /// The registry host, `"docker.io"` when host is `None`.
    pub fn host_str(&self) -> &str {
        self.host.as_deref().unwrap_or("docker.io")
    }

    /// True for Docker Hub: host `None`, `"docker.io"` or `"index.docker.io"`.
    pub fn is_docker_hub(&self) -> bool {
        matches!(
            self.host.as_deref(),
            None | Some("docker.io") | Some("index.docker.io")
        )
    }

    /// The tag, `"latest"` when tag is `None`.
    pub fn effective_tag(&self) -> &str {
        self.tag.as_deref().unwrap_or("latest")
    }

    /// Repo path for the registry API: Docker Hub short names like `"alpine"`
    /// become `"library/alpine"`; anything else is returned as-is.
    pub fn api_repo(&self) -> String {
        if self.is_docker_hub() && !self.repo.contains('/') {
            format!("library/{}", self.repo)
        } else {
            self.repo.clone()
        }
    }

    /// URL base key for registry lookups; always `host_str()` for M1.
    pub fn base_url_key(&self) -> &str {
        self.host_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn parse_table() {
        let cases: &[(&str, Option<&str>, &str, Option<&str>, Option<&str>)] = &[
            ("alpine", None, "alpine", None, None),
            ("alpine:3.20", None, "alpine", Some("3.20"), None),
            (
                "docker.io/alpine:3.20",
                Some("docker.io"),
                "alpine",
                Some("3.20"),
                None,
            ),
            (
                "library/alpine:3.20",
                None,
                "library/alpine",
                Some("3.20"),
                None,
            ),
            (
                "ghcr.io/org/img:v1",
                Some("ghcr.io"),
                "org/img",
                Some("v1"),
                None,
            ),
            (
                "reg.example.com:5000/a/b:1",
                Some("reg.example.com:5000"),
                "a/b",
                Some("1"),
                None,
            ),
            // Note: brief's `concat!("alpine:3.20@", DIGEST)` does not compile (concat! only
            // accepts literals); inlined the literal instead — identical test input.
            (
                "alpine:3.20@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                None,
                "alpine",
                Some("3.20"),
                Some(DIGEST),
            ),
        ];
        for (input, host, repo, tag, digest) in cases {
            let r = ImageRef::parse(input).unwrap();
            assert_eq!(r.host.as_deref(), *host, "host: {input}");
            assert_eq!(r.repo, *repo, "repo: {input}");
            assert_eq!(r.tag.as_deref(), *tag, "tag: {input}");
            assert_eq!(r.digest.as_deref(), *digest, "digest: {input}");
        }
    }

    #[test]
    fn parse_rejects_garbage() {
        for bad in [
            "",
            "a b",
            "alpine:3.20@notadigest",
            "alpine:3.20@sha256:zzz",
            "@sha256:0000000000000000000000000000000000000000000000000000000000000000",
        ] {
            assert!(ImageRef::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn classification() {
        assert!(ImageRef::parse("alpine").unwrap().is_docker_hub());
        assert!(ImageRef::parse("docker.io/x/y:1").unwrap().is_docker_hub());
        assert!(!ImageRef::parse("ghcr.io/x/y:1").unwrap().is_docker_hub());
        assert!(!ImageRef::parse("reg:5000/x/y:1").unwrap().is_docker_hub());
        assert_eq!(
            ImageRef::parse("alpine").unwrap().api_repo(),
            "library/alpine"
        );
        assert_eq!(
            ImageRef::parse("library/alpine").unwrap().api_repo(),
            "library/alpine"
        );
        assert_eq!(ImageRef::parse("org/img").unwrap().api_repo(), "org/img");
        assert_eq!(ImageRef::parse("alpine").unwrap().effective_tag(), "latest");
    }
}
