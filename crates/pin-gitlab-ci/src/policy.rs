//! gitlab-ci policy for components and includes (spec §6.2): skip rules and
//! host gate first (no client is built before it is needed), then
//! resolution through `Ctx::gitlab_resolve` only (D38).

use pin_core::edit::{Splice, requote, trailing_comment};
use pin_core::git::is_commit_sha;
use pin_core::plugin::Finding;
use pin_core::report::Status;
use pin_core::run::Ctx;

use crate::discover::PinTarget;
use crate::is_catalog_managed;

/// The finding skeleton for `t`: `reference` is the discovered value, `old`
/// the token as written.
fn base(text: &str, t: &PinTarget, kind: &'static str) -> Finding {
    Finding {
        line: t.line(),
        kind,
        reference: t.value().to_string(),
        old: text[t.start()..t.end()].to_string(),
        new: None,
        // Placeholder: every policy path sets the real status.
        status: Status::Unresolved,
        reason: None,
        edits: Vec::new(),
        follow_up: None,
    }
}

fn with(mut f: Finding, status: Status, reason: Option<String>) -> Finding {
    f.status = status;
    f.reason = reason;
    f
}

/// `f` pinned to `new_value` (re-quoted like the token), plus the trailing
/// `  # {orig}` comment when the line has room for one (M3 constraint 4).
fn pinned(mut f: Finding, text: &str, t: &PinTarget, new_value: &str, orig: &str) -> Finding {
    let new_token = requote(&text[t.start()..t.end()], new_value);
    f.status = Status::Pinned;
    f.edits = std::iter::once(Splice::replace(t.start(), t.end(), new_token.clone()))
        .chain(trailing_comment(text, t.line(), t.end(), orig))
        .collect();
    f.new = Some(new_token);
    f
}

/// A component target → `@<sha>` (M3 Task 4 rules): unparseable, host not
/// in `gitlab_hosts`, already a SHA, or catalog-managed → no resolve.
pub(crate) async fn resolve_component(
    gitlab_hosts: &[String],
    ctx: &Ctx,
    text: &str,
    t: &PinTarget,
) -> Finding {
    let PinTarget::Component { parts, .. } = t else {
        unreachable!("resolve_component receives Component targets")
    };
    let f = base(text, t, "component");
    let Some(parts) = parts else {
        return with(f, Status::Skipped, Some("unparseable reference".into()));
    };
    if !gitlab_hosts.contains(&parts.host) {
        let reason = format!("host not enabled: {}", parts.host);
        return with(f, Status::Skipped, Some(reason));
    }
    if is_commit_sha(&parts.version) {
        return with(f, Status::Ok, None);
    }
    if is_catalog_managed(&parts.version) {
        return with(f, Status::Skipped, Some("catalog-managed version".into()));
    }
    match ctx
        .gitlab_resolve(Some(&parts.host), &parts.project, &parts.version)
        .await
    {
        Err(e) => with(f, Status::Unresolved, Some(e)),
        Ok(sha) => {
            let new_value = format!("{}/{}/{}@{}", parts.host, parts.project, parts.name, sha);
            pinned(f, text, t, &new_value, &parts.version)
        }
    }
}

/// A classic-include `ref:` target → commit SHA on the primary GitLab host
/// (M3 Task 4 rules): already a SHA or templated → no resolve.
pub(crate) async fn resolve_include(ctx: &Ctx, text: &str, t: &PinTarget) -> Finding {
    let PinTarget::IncludeRef { value, project, .. } = t else {
        unreachable!("resolve_include receives IncludeRef targets")
    };
    let f = base(text, t, "include");
    if is_commit_sha(value) {
        return with(f, Status::Ok, None);
    }
    if value.contains('$') {
        return with(f, Status::Skipped, Some("templated reference".into()));
    }
    match ctx.gitlab_resolve(None, project, value).await {
        Err(e) => with(f, Status::Unresolved, Some(e)),
        Ok(sha) => pinned(f, text, t, &sha, value),
    }
}
