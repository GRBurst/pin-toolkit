pub mod discover;
pub mod edit;
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

/// One resolved target: the report row plus the
/// `(PinTarget::start(), new token)` splice when the file should change.
struct Resolved {
    result: RefResult,
    replacement: Option<(usize, String)>,
}

/// Resolve one discovered target against the client.
async fn resolve_target(
    client: &RegistryClient,
    opts: &PinOptions,
    text: &str,
    t: &PinTarget,
    file: &str,
) -> Resolved {
    let token = &text[t.start()..t.end()];
    let mut result = RefResult {
        file: file.to_string(),
        line: t.line() + 1,
        reference: t.value().to_string(),
        old: token.to_string(),
        new: None,
        status: String::new(),
        reason: None,
    };
    let PinTarget::Image { .. } = t else {
        // Component and include targets are resolved against GitLab in
        // M3 Task 4; until then they are reported, not pinned. Unparseable
        // component values use the same reason as image references.
        result.status = "skipped".to_string();
        result.reason = Some(if matches!(t, PinTarget::Component { parts: None, .. }) {
            "unparseable reference".to_string()
        } else if matches!(t, PinTarget::IncludeRef { .. }) {
            "include resolution pending".to_string()
        } else {
            "component resolution pending".to_string()
        });
        return Resolved {
            result,
            replacement: None,
        };
    };
    let r = match ImageRef::parse(t.value()) {
        Ok(r) => r,
        Err(_) => {
            result.status = "skipped".to_string();
            result.reason = Some("unparseable reference".to_string());
            return Resolved {
                result,
                replacement: None,
            };
        }
    };
    if !r.is_docker_hub() {
        result.status = "skipped".to_string();
        result.reason = Some(format!("registry not enabled: {}", r.host_str()));
        return Resolved {
            result,
            replacement: None,
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
            };
        }
    };
    if r.digest.as_deref() == Some(digest.as_str()) {
        result.status = "ok".to_string();
        return Resolved {
            result,
            replacement: None,
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
                        };
                    }
                };
                let new_token = edit::splice_update(token, &new_tag, &new_digest);
                result.status = "updated".to_string();
                result.new = Some(new_token.clone());
                return Resolved {
                    result,
                    replacement: Some((t.start(), new_token)),
                };
            }
        }
    }
    let new_token = edit::splice_value(token, &r, &digest);
    result.status = "pinned".to_string();
    result.new = Some(new_token.clone());
    Resolved {
        result,
        replacement: Some((t.start(), new_token)),
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

/// Resolve every image target in one CI file against `client`, apply the
/// edits (unless `opts.dry_run`), and write the file only when at least one
/// edit was applied. Results are sorted by line.
pub async fn pin_file(
    path: &Path,
    opts: &PinOptions,
    client: &RegistryClient,
) -> Result<Vec<RefResult>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let targets = discover(&text);
    let file = path.display().to_string();

    let limit = opts.concurrency.max(1);
    let sem = Arc::new(Semaphore::new(limit));
    let text_ref: &str = &text;
    let file_ref: &str = &file;
    let client_ref: &RegistryClient = client;
    let mut futures: Vec<_> = Vec::with_capacity(targets.len());
    for t in &targets {
        let sem = Arc::clone(&sem);
        futures.push(async move {
            let _permit = sem.acquire().await.expect("semaphore is never closed");
            resolve_target(client_ref, opts, text_ref, t, file_ref).await
        });
    }
    let mut resolved: Vec<Resolved> = join_all(futures).await;
    resolved.sort_by_key(|r| r.result.line);

    let mut replacements: BTreeMap<usize, String> = BTreeMap::new();
    for r in &resolved {
        if let Some((start, new_token)) = &r.replacement {
            replacements.insert(*start, new_token.clone());
        }
    }
    if !opts.dry_run && !replacements.is_empty() {
        let (new_text, applied) = edit::apply(&text, &targets, &replacements);
        if applied > 0 {
            std::fs::write(path, new_text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(resolved.into_iter().map(|r| r.result).collect())
}

/// Pin every target in each file (files in order; resolutions within one
/// file run concurrently, bounded by `opts.concurrency`).
pub async fn pin_files(
    paths: &[PathBuf],
    opts: &PinOptions,
    client: &RegistryClient,
) -> Result<Report, String> {
    let mut results = Vec::new();
    for path in paths {
        results.extend(pin_file(path, opts, client).await?);
    }
    Ok(Report::new(results))
}

/// Exit code for a report: 1 when anything was unresolved, else 0.
pub fn exit_code(report: &Report) -> i32 {
    if report.summary.unresolved > 0 { 1 } else { 0 }
}
