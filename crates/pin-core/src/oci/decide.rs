//! Image policy in core (spec §6.2): today's `resolve_image` as a decision
//! plus the inline-image finding. Resolvers are reached only through the
//! `Ctx` entry points (D38), so every error text is today's.

use crate::edit::Splice;
use crate::oci::registry::RegError;
use crate::oci::{ImageRef, splice_update, splice_value};
use crate::plugin::Finding;
use crate::report::Status;
use crate::run::{AgePolicy, Ctx};
use crate::ver::age::is_old_enough;
use crate::ver::docker as versioning;

/// What to do with one image reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageDecision {
    /// Already pinned to the current digest.
    Current,
    /// Pin the current tag's digest.
    Pin { digest: String },
    /// Bump to `tag` and pin its digest (`--update`).
    Update { tag: String, digest: String },
    /// Leave unchanged, with a reason.
    Skip(String),
}

/// The inner message of a registry error (no `unresolved:` prefix).
fn inner(e: RegError) -> String {
    match e {
        RegError::Unresolved(m) | RegError::Network(m) => m,
    }
}

/// Decide `r` (M1 pin, M2 update, M4b-1 release-age guard). `Err` is the
/// exact `unresolved` reason: digest errors report the inner message,
/// tag-list errors read `tag list: {e}`, Hub API errors start `hub api:`.
pub async fn decide_image(ctx: &Ctx, r: &ImageRef) -> Result<ImageDecision, String> {
    let digest = ctx.oci_digest(r).await.map_err(inner)?;
    if r.digest.as_deref() == Some(digest.as_str()) {
        return Ok(ImageDecision::Current);
    }
    if ctx.update.enabled && r.digest.is_none() {
        // Pinned refs never reach here (re-pin only); only non-empty,
        // versioned tags bump — tagless and non-semver tags fall through to
        // the M1 pin path.
        if let Some(tag) = r.tag.as_deref()
            && !tag.is_empty()
            && versioning::parse(tag).is_some()
        {
            let tags = ctx
                .oci_tags(r)
                .await
                .map_err(|e| format!("tag list: {e}"))?;
            let allow_major = ctx.update.allow_major;
            let candidate = versioning::pick_latest_stable(tag, &tags, allow_major);
            // M4b-1 release-age guard (R3-R6): active iff `min_age_days > 0`
            // AND (the ref is docker.io OR `release_age.unknown = block`,
            // §6.8) — otherwise every other host stays pure M2.
            // Double-pick: the unfiltered M2 candidate vs the candidate
            // picked from the tags old enough; a tag missing from the Hub
            // timestamp map is NOT old enough (timestamp-required parity).
            let guard = ctx.update.min_age_days > 0
                && (r.is_docker_hub() || ctx.update.unknown_age == AgePolicy::Block);
            let new_tag = if guard && !r.is_docker_hub() {
                // No timestamps outside docker.io (no request): the age of
                // an update candidate is unknown, so `block` skips it; no
                // candidate → the M2 no-update path (pin).
                match candidate {
                    Some(c) if c != tag => {
                        return Ok(ImageDecision::Skip(format!(
                            "release age unknown ({}; release_age.unknown = block)",
                            r.host_str()
                        )));
                    }
                    _ => None,
                }
            } else if guard {
                let stamps = ctx.oci_tag_times(r).await.map_err(|e| {
                    // tag_timestamps already prefixes its own errors with
                    // "hub api:"; the bare network message gets the prefix
                    // here (plan constraint 5).
                    let msg = inner(e);
                    if msg.starts_with("hub api:") {
                        msg
                    } else {
                        format!("hub api: {msg}")
                    }
                })?;
                let now = ctx.now_unix;
                let days = u64::from(ctx.update.min_age_days);
                let tags_age: Vec<String> = tags
                    .iter()
                    .filter(|t| {
                        stamps
                            .get(t.as_str())
                            .is_some_and(|ts| is_old_enough(*ts, now, days))
                    })
                    .cloned()
                    .collect();
                let age_candidate = versioning::pick_latest_stable(tag, &tags_age, allow_major);
                let too_young =
                    || ImageDecision::Skip(format!("release age: no candidate ≥ {days}d"));
                match (candidate, age_candidate) {
                    // R5: unfiltered candidates existed but none is old
                    // enough → skip; the ref stays byte-identical, exit 0.
                    (Some(c), None) if c != tag => return Ok(too_young()),
                    (Some(c), Some(s)) if c != tag && s == tag => return Ok(too_young()),
                    // An old-enough candidate exists → update to IT.
                    (Some(_), Some(a)) if a != tag => Some(a),
                    // No unfiltered candidate: the M2 no-update path (pin).
                    _ => None,
                }
            } else {
                candidate
            };
            if let Some(new_tag) = new_tag
                && new_tag != tag
            {
                let mut bumped = r.clone();
                bumped.tag = Some(new_tag.clone());
                let digest = ctx.oci_digest(&bumped).await.map_err(inner)?;
                return Ok(ImageDecision::Update {
                    tag: new_tag,
                    digest,
                });
            }
        }
    }
    Ok(ImageDecision::Pin { digest })
}

/// The finding for an inline image reference `value` whose token (as
/// written, quotes included) is `text[start..end]` on 0-based `line`:
/// unparseable → skipped without touching a resolver; otherwise
/// [`decide_image`] mapped to one quote-preserving replacement.
pub async fn pin_inline_image(
    ctx: &Ctx,
    text: &str,
    line: usize,
    start: usize,
    end: usize,
    value: &str,
    kind: &'static str,
) -> Finding {
    let token = &text[start..end];
    let mut f = Finding {
        line,
        kind,
        reference: value.to_string(),
        old: token.to_string(),
        new: None,
        status: Status::Unresolved,
        reason: None,
        edits: Vec::new(),
        follow_up: None,
    };
    let r = match ImageRef::parse(value) {
        Ok(r) => r,
        Err(_) => {
            f.status = Status::Skipped;
            f.reason = Some("unparseable reference".to_string());
            return f;
        }
    };
    let (status, new_token) = match decide_image(ctx, &r).await {
        Ok(ImageDecision::Current) => (Status::Ok, None),
        Ok(ImageDecision::Pin { digest }) => {
            (Status::Pinned, Some(splice_value(token, &r, &digest)))
        }
        Ok(ImageDecision::Update { tag, digest }) => {
            (Status::Updated, Some(splice_update(token, &tag, &digest)))
        }
        Ok(ImageDecision::Skip(reason)) => {
            f.reason = Some(reason);
            (Status::Skipped, None)
        }
        Err(msg) => {
            f.reason = Some(msg);
            (Status::Unresolved, None)
        }
    };
    f.status = status;
    if let Some(t) = new_token {
        f.edits = vec![Splice::replace(start, end, t.clone())];
        f.new = Some(t);
    }
    f
}
