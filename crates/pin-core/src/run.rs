//! One file through one plugin (spec §6.2/§6.3): read, plan (scan → bounded
//! resolve), overlap check, write, rows. `Ctx` carries the run's policy.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::edit::{Splice, apply};
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
    pub home: Option<PathBuf>,
    pub gitlab_host: String,
}

/// Per-run context: policy, clock and (from C7) lazily built resolvers.
pub struct Ctx {
    pub update: UpdatePolicy,
    pub now_unix: i64,
    env: ResolverEnv,
}

impl Ctx {
    /// Builds nothing: resolvers are created on first use.
    pub fn new(update: UpdatePolicy, now_unix: i64, env: ResolverEnv) -> Self {
        Self {
            update,
            now_unix,
            env,
        }
    }

    pub fn env(&self) -> &ResolverEnv {
        &self.env
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

    let splices: Vec<Splice> = findings.iter().flat_map(|f| f.edits.clone()).collect();
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
