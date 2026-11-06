pub mod discover;
pub mod edit;
pub mod gitlab;
pub mod image_ref;
pub mod registry;
pub mod report;
pub mod versioning;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;

use tokio::sync::Semaphore;

use crate::discover::{PinTarget, discover};
use crate::gitlab::GitLabClient;
use crate::registry::{Creds, RegError};
use crate::report::{RefResult, Report};

pub use image_ref::ImageRef;
pub use registry::RegistryClient;

/// Options for a pin run.
#[derive(Debug, Clone)]
pub struct PinOptions {
    /// Resolve and report without writing files.
    pub dry_run: bool,
    /// Report as JSON (rendering choice, handled by the CLI).
    pub json: bool,
    /// Max concurrent digest lookups per file.
    pub concurrency: usize,
    /// Bump unpinned, versioned Docker Hub refs to the latest compatible
    /// stable tag and pin the bumped tag's digest (status `updated`).
    pub update: bool,
    /// With `update`, allow bumping to a higher major (same suffix and
    /// release length still required). Meaningful only when `update` is set.
    pub update_major: bool,
    /// GitLab instance host for component/include resolution (default
    /// `gitlab.com`; the `--gitlab-host` flag is wired in M3 Task 5).
    pub gitlab_host: String,
}

/// Build the registry client from the environment: `DOCKERHUB_USER` /
/// `DOCKERHUB_TOKEN` (falling back to `~/.docker/config.json` via
/// [`Creds::load_from`]); the default Docker Hub base URL is used when no
/// registry is configured.
pub fn client_from_env() -> RegistryClient {
    let creds = Creds::load_from(
        std::env::var("DOCKERHUB_USER").ok(),
        std::env::var("DOCKERHUB_TOKEN").ok(),
        std::env::var("HOME").ok().as_deref().map(Path::new),
    );
    RegistryClient::new(creds, HashMap::new())
}

/// Build the GitLab client for instance `host` from the environment
/// (base `https://{host}/api/v4`); `GITLAB_TOKEN` takes precedence over
/// `CI_JOB_TOKEN` (M3 Task 3).
pub fn gitlab_from_env(host: &str) -> GitLabClient {
    GitLabClient::from_env(format!("https://{host}/api/v4"))
}

/// One resolved target: the report row plus the splices that should
/// change the file.
struct Resolved {
    result: RefResult,
    /// `(start, end, new_token)` replacement when the file should change.
    replacement: Option<(usize, usize, String)>,
    /// Trailing-comment splice `(pos, "  # orig")` for pinned component/
    /// include results (M3 constraint 4), else `None`.
    comment: Option<(usize, String)>,
}

/// The base report row for a target. `reference` is the discovered value:
/// the image ref, the FULL component value, or the include's ref value.
fn base_result(text: &str, t: &PinTarget, file: &str) -> RefResult {
    RefResult {
        file: file.to_string(),
        line: t.line() + 1,
        reference: t.value().to_string(),
        old: text[t.start()..t.end()].to_string(),
        new: None,
        status: String::new(),
        reason: None,
    }
}

/// A 40-hex lowercase commit SHA (M3 constraint 7): already pinned.
fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A component version GitLab resolves via the CI/CD Catalog at pipeline
/// time (M3 constraint 8): starts with `~`, is numeric-only
/// (`^\d+(\.\d+)*$`), or contains `.x`. These have no tag/branch to pin
/// against, so they are skipped, not resolved.
fn is_catalog_managed(version: &str) -> bool {
    if version.starts_with('~') || version.contains(".x") {
        return true;
    }
    let mut parts = version.split('.');
    match parts.next() {
        Some(first) if !first.is_empty() && first.bytes().all(|b| b.is_ascii_digit()) => {}
        _ => return false,
    }
    parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The trailing-comment splice for a pinned component/include result
/// (M3 constraint 4): `Some((pos, "  # orig"))` when the token's line has
/// room for a trailing comment; `None` when a `#` already occurs after the
/// token on the same line (no double comment).
fn comment_splice(text: &str, t: &PinTarget, orig: &str) -> Option<(usize, String)> {
    edit::comment_insert_offset(text, t.line(), t.end()).map(|pos| (pos, format!("  # {orig}")))
}

/// Resolve one discovered target: images against the registry (M1/M2),
/// components and includes against GitLab (M3 Task 4).
async fn resolve_target(
    reg: &RegistryClient,
    gitlab: &GitLabClient,
    opts: &PinOptions,
    text: &str,
    t: &PinTarget,
    file: &str,
) -> Resolved {
    match t {
        PinTarget::Image { .. } => resolve_image(reg, opts, text, t, file).await,
        PinTarget::Component { .. } => resolve_component(gitlab, opts, text, t, file).await,
        PinTarget::IncludeRef { .. } => resolve_include(gitlab, text, t, file).await,
    }
}

/// Resolve an image target against the registry (M1/M2 behavior, unchanged).
async fn resolve_image(
    client: &RegistryClient,
    opts: &PinOptions,
    text: &str,
    t: &PinTarget,
    file: &str,
) -> Resolved {
    let token = &text[t.start()..t.end()];
    let mut result = base_result(text, t, file);
    let r = match ImageRef::parse(t.value()) {
        Ok(r) => r,
        Err(_) => {
            result.status = "skipped".to_string();
            result.reason = Some("unparseable reference".to_string());
            return Resolved {
                result,
                replacement: None,
                comment: None,
            };
        }
    };
    if !r.is_docker_hub() {
        result.status = "skipped".to_string();
        result.reason = Some(format!("registry not enabled: {}", r.host_str()));
        return Resolved {
            result,
            replacement: None,
            comment: None,
        };
    }
    let digest = match client.digest(&r).await {
        Ok(d) => d,
        Err(e) => {
            let msg = match e {
                RegError::Unresolved(m) | RegError::Network(m) => m,
            };
            result.status = "unresolved".to_string();
            result.reason = Some(msg);
            return Resolved {
                result,
                replacement: None,
                comment: None,
            };
        }
    };
    if r.digest.as_deref() == Some(digest.as_str()) {
        result.status = "ok".to_string();
        return Resolved {
            result,
            replacement: None,
            comment: None,
        };
    }
    if opts.update && r.digest.is_none() {
        // Pinned refs never reach here (re-pin only); only non-empty,
        // versioned tags bump — tagless and non-semver tags fall through to
        // the M1 pin path.
        if let Some(tag) = r.tag.as_deref()
            && !tag.is_empty()
            && versioning::parse(tag).is_some()
        {
            let tags = match client.list_tags(&r).await {
                Ok(tags) => tags,
                Err(e) => {
                    result.status = "unresolved".to_string();
                    result.reason = Some(format!("tag list: {e}"));
                    return Resolved {
                        result,
                        replacement: None,
                        comment: None,
                    };
                }
            };
            if let Some(new_tag) = versioning::pick_latest_stable(tag, &tags, opts.update_major)
                && new_tag != tag
            {
                let mut bumped = r.clone();
                bumped.tag = Some(new_tag.clone());
                let new_digest = match client.digest(&bumped).await {
                    Ok(d) => d,
                    Err(e) => {
                        let msg = match e {
                            RegError::Unresolved(m) | RegError::Network(m) => m,
                        };
                        result.status = "unresolved".to_string();
                        result.reason = Some(msg);
                        return Resolved {
                            result,
                            replacement: None,
                            comment: None,
                        };
                    }
                };
                let new_token = edit::splice_update(token, &new_tag, &new_digest);
                result.status = "updated".to_string();
                result.new = Some(new_token.clone());
                return Resolved {
                    result,
                    replacement: Some((t.start(), t.end(), new_token)),
                    comment: None,
                };
            }
        }
    }
    let new_token = edit::splice_value(token, &r, &digest);
    result.status = "pinned".to_string();
    result.new = Some(new_token.clone());
    Resolved {
        result,
        replacement: Some((t.start(), t.end(), new_token)),
        comment: None,
    }
}

/// Resolve a component target to a pinned `@<sha>` (M3 Task 4 GREEN rules).
async fn resolve_component(
    gitlab: &GitLabClient,
    opts: &PinOptions,
    text: &str,
    t: &PinTarget,
    file: &str,
) -> Resolved {
    let PinTarget::Component { parts, .. } = t else {
        unreachable!("resolve_component receives Component targets")
    };
    let mut result = base_result(text, t, file);
    let parts = match parts {
        Some(p) => p,
        None => {
            result.status = "skipped".to_string();
            result.reason = Some("unparseable reference".to_string());
            return Resolved {
                result,
                replacement: None,
                comment: None,
            };
        }
    };
    if parts.host != opts.gitlab_host {
        result.status = "skipped".to_string();
        result.reason = Some(format!("host not enabled: {}", parts.host));
        return Resolved {
            result,
            replacement: None,
            comment: None,
        };
    }
    if is_sha(&parts.version) {
        result.status = "ok".to_string();
        return Resolved {
            result,
            replacement: None,
            comment: None,
        };
    }
    if is_catalog_managed(&parts.version) {
        result.status = "skipped".to_string();
        result.reason = Some("catalog-managed version".to_string());
        return Resolved {
            result,
            replacement: None,
            comment: None,
        };
    }
    let sha = match gitlab.resolve(&parts.project, &parts.version).await {
        Ok(sha) => sha,
        Err(e) => {
            result.status = "unresolved".to_string();
            result.reason = Some(e.to_string());
            return Resolved {
                result,
                replacement: None,
                comment: None,
            };
        }
    };
    let token = &text[t.start()..t.end()];
    let new_value = format!("{}/{}/{}@{}", parts.host, parts.project, parts.name, sha);
    let new_token = edit::splice_replace(token, &new_value);
    result.status = "pinned".to_string();
    result.new = Some(new_token.clone());
    Resolved {
        result,
        replacement: Some((t.start(), t.end(), new_token)),
        comment: comment_splice(text, t, &parts.version),
    }
}

/// Resolve a classic-include `ref:` target to a commit SHA (M3 Task 4
/// GREEN rules).
async fn resolve_include(gitlab: &GitLabClient, text: &str, t: &PinTarget, file: &str) -> Resolved {
    let PinTarget::IncludeRef { value, project, .. } = t else {
        unreachable!("resolve_include receives IncludeRef targets")
    };
    let mut result = base_result(text, t, file);
    if is_sha(value) {
        result.status = "ok".to_string();
        return Resolved {
            result,
            replacement: None,
            comment: None,
        };
    }
    if value.contains('$') {
        result.status = "skipped".to_string();
        result.reason = Some("templated reference".to_string());
        return Resolved {
            result,
            replacement: None,
            comment: None,
        };
    }
    let sha = match gitlab.resolve(project, value).await {
        Ok(sha) => sha,
        Err(e) => {
            result.status = "unresolved".to_string();
            result.reason = Some(e.to_string());
            return Resolved {
                result,
                replacement: None,
                comment: None,
            };
        }
    };
    let token = &text[t.start()..t.end()];
    let new_token = edit::splice_replace(token, &sha);
    result.status = "pinned".to_string();
    result.new = Some(new_token.clone());
    Resolved {
        result,
        replacement: Some((t.start(), t.end(), new_token)),
        comment: comment_splice(text, t, value),
    }
}

/// Drive all `futures` concurrently on the current task until each is
/// complete, returning results in input order.
///
/// A stand-in for `futures::future::join_all` so the crate adds no
/// dependencies for this.
async fn join_all<F>(futures: Vec<F>) -> Vec<F::Output>
where
    F: Future,
{
    let mut slots: Vec<Option<std::pin::Pin<Box<F>>>> =
        futures.into_iter().map(|f| Some(Box::pin(f))).collect();
    let mut results: Vec<Option<F::Output>> = (0..slots.len()).map(|_| None).collect();
    poll_fn(|cx| {
        let mut any_pending = false;
        for (i, slot) in slots.iter_mut().enumerate() {
            if let Some(fut) = slot {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(out) => {
                        results[i] = Some(out);
                        *slot = None;
                    }
                    Poll::Pending => any_pending = true,
                }
            }
        }
        if any_pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    results
        .into_iter()
        .map(|r| r.expect("completed future has a result"))
        .collect()
}

/// Resolve every target in one CI file (images against the registry, M3
/// components/includes against GitLab), apply the edits (unless
/// `opts.dry_run`), and write the file only when at least one splice was
/// applied. Results are sorted by line.
pub async fn pin_file(
    path: &Path,
    opts: &PinOptions,
    reg: &RegistryClient,
    gitlab: &GitLabClient,
) -> Result<Vec<RefResult>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let targets = discover(&text);
    let file = path.display().to_string();

    // Registry digest lookups and GitLab ref resolutions run in parallel,
    // under the same semaphore.
    let limit = opts.concurrency.max(1);
    let sem = Arc::new(Semaphore::new(limit));
    let text_ref: &str = &text;
    let file_ref: &str = &file;
    let reg_ref: &RegistryClient = reg;
    let gitlab_ref: &GitLabClient = gitlab;
    let mut futures: Vec<_> = Vec::with_capacity(targets.len());
    for t in &targets {
        let sem = Arc::clone(&sem);
        futures.push(async move {
            let _permit = sem.acquire().await.expect("semaphore is never closed");
            resolve_target(reg_ref, gitlab_ref, opts, text_ref, t, file_ref).await
        });
    }
    let mut resolved: Vec<Resolved> = join_all(futures).await;
    resolved.sort_by_key(|r| r.result.line);

    // Splices: replacements (deduped by start, as before) plus trailing
    // comment insertions for pinned components/includes.
    let mut replacements: BTreeMap<usize, (usize, String)> = BTreeMap::new();
    for r in &resolved {
        if let Some((start, end, new_token)) = &r.replacement {
            replacements.insert(*start, (*end, new_token.clone()));
        }
    }
    let mut splices: Vec<(usize, usize, String)> = replacements
        .into_iter()
        .map(|(s, (e, n))| (s, e, n))
        .collect();
    for r in &resolved {
        if let Some((pos, comment)) = &r.comment {
            splices.push((*pos, *pos, comment.clone()));
        }
    }
    if !opts.dry_run && !splices.is_empty() {
        let (new_text, _) = edit::apply_splices(&text, &splices);
        std::fs::write(path, new_text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(resolved.into_iter().map(|r| r.result).collect())
}

/// Pin every target in each file (files in order; resolutions within one
/// file run concurrently, bounded by `opts.concurrency`).
pub async fn pin_files(
    paths: &[PathBuf],
    opts: &PinOptions,
    reg: &RegistryClient,
    gitlab: &GitLabClient,
) -> Result<Report, String> {
    let mut results = Vec::new();
    for path in paths {
        results.extend(pin_file(path, opts, reg, gitlab).await?);
    }
    Ok(Report::new(results))
}

/// Exit code for a report: 1 when anything was unresolved, else 0.
pub fn exit_code(report: &Report) -> i32 {
    if report.summary.unresolved > 0 { 1 } else { 0 }
}
