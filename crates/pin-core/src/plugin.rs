//! The plugin contract (spec §6.2): a plugin owns *where* targets are (a pure
//! `scan`) and *policy* (`resolve` turns resolver answers into a [`Finding`]
//! with splices); the core owns resolvers ([`Ctx`]), edits, I/O and the
//! report.

use std::collections::BTreeSet;
use std::future::{Future, poll_fn};
use std::path::Path;
use std::pin::Pin;
use std::task::Poll;

use tokio::sync::Semaphore;

use crate::edit::Splice;
use crate::report::Status;
use crate::run::Ctx;
use crate::text::LineIndex;

/// A boxed future that need not be `Send` (futures are joined, never spawned).
pub type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// One kind of reference a plugin finds (e.g. gitlab-ci `image`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindSpec {
    pub id: &'static str,
    pub default_on: bool,
    pub help: &'static str,
}

/// The enabled kind ids of one plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KindSet(BTreeSet<&'static str>);

impl KindSet {
    /// Every kind whose `default_on` is set.
    pub fn defaults(kinds: &[KindSpec]) -> Self {
        Self(
            kinds
                .iter()
                .filter(|k| k.default_on)
                .map(|k| k.id)
                .collect(),
        )
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.contains(id)
    }

    pub fn set(&mut self, id: &'static str, on: bool) {
        if on {
            self.0.insert(id);
        } else {
            self.0.remove(id);
        }
    }
}

/// `ignore` patterns (spec §7.1, T-D17) over a plugin's
/// [`Plugin::ignore_key`]: `*` matches any run of characters (incl. `/`,
/// incl. none), `?` exactly one; a pattern matches the whole key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefPatterns(Vec<String>);

impl RefPatterns {
    pub fn new(patterns: impl IntoIterator<Item = String>) -> Self {
        Self(patterns.into_iter().collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Some pattern matches the whole `key`.
    pub fn matches(&self, key: &str) -> bool {
        let key: Vec<char> = key.chars().collect();
        self.0.iter().any(|p| {
            let p: Vec<char> = p.chars().collect();
            wildcard(&p, &key)
        })
    }
}

/// Whole-string wildcard match (`*`, `?`) over chars: two pointers,
/// backtracking to the last `*`.
fn wildcard(p: &[char], s: &[char]) -> bool {
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < s.len() {
        if i < p.len() && (p[i] == '?' || p[i] == s[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some((i, j));
            i += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

/// What one plugin plans in a file: the enabled kinds and the `ignore`
/// patterns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub kinds: KindSet,
    pub ignore: RefPatterns,
}

impl From<KindSet> for Selection {
    /// `kinds`, nothing ignored.
    fn from(kinds: KindSet) -> Self {
        Self {
            kinds,
            ignore: RefPatterns::default(),
        }
    }
}

/// One reference, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// 0-based line; the sort key.
    pub line: usize,
    pub kind: &'static str,
    pub reference: String,
    pub old: String,
    pub new: Option<String>,
    pub status: Status,
    pub reason: Option<String>,
    /// Empty unless pinned/updated.
    pub edits: Vec<Splice>,
    /// E.g. `helm dependency update charts/app` (M8b; dropped by `pin_file`).
    pub follow_up: Option<String>,
}

/// The file does not parse (§6.9): no partial targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    /// 1-based line, when known.
    pub line: Option<usize>,
    pub msg: String,
}

/// One file as `resolve` sees it (review L3): path, text and its
/// [`LineIndex`], built once per file by [`DynPlugin::plan`].
#[derive(Debug, Clone)]
pub struct Source<'a> {
    pub path: &'a Path,
    pub text: &'a str,
    pub lines: LineIndex,
}

impl<'a> Source<'a> {
    pub fn new(path: &'a Path, text: &'a str) -> Self {
        Self {
            path,
            text,
            lines: LineIndex::new(text),
        }
    }
}

/// Identity shared by [`Plugin`] and [`DynPlugin`]: one method set, so both
/// traits can be in scope without ambiguity.
pub trait PluginMeta {
    /// `"gitlab-ci"`: config key, subcommand, report field.
    fn id(&self) -> &'static str;
    fn kinds(&self) -> &'static [KindSpec];
    /// Globs.
    fn default_files(&self) -> &'static [&'static str];
}

/// A plugin: a pure scan plus per-target policy.
#[allow(async_fn_in_trait)] // no Send bound by design (§6.2): futures are joined, never spawned
pub trait Plugin: PluginMeta {
    type Target;
    /// Pure; targets sorted by (line, start); `Err` = the file does not parse.
    fn scan(&self, path: &Path, text: &str, on: &KindSet) -> Result<Vec<Self::Target>, ScanError>;
    /// The key `ignore` patterns match (spec §7.1); `None` = never
    /// ignored.
    fn ignore_key(&self, _t: &Self::Target) -> Option<String> {
        None
    }
    /// `src` is the scanned file (one [`LineIndex`] per file).
    async fn resolve(&self, ctx: &Ctx, src: &Source<'_>, t: &Self::Target) -> Finding;
}

/// Object-safe face of [`Plugin`]: scan the selected kinds, drop the
/// targets `sel.ignore` matches (never resolved), then resolve every target
/// with at most `concurrency` (min 1) in flight; findings in target order.
pub trait DynPlugin: PluginMeta {
    fn plan<'a>(
        &'a self,
        ctx: &'a Ctx,
        path: &'a Path,
        text: &'a str,
        sel: &'a Selection,
        concurrency: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<Finding>, ScanError>>;
}

impl<P: Plugin> DynPlugin for P {
    fn plan<'a>(
        &'a self,
        ctx: &'a Ctx,
        path: &'a Path,
        text: &'a str,
        sel: &'a Selection,
        concurrency: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<Finding>, ScanError>> {
        Box::pin(async move {
            let mut targets = self.scan(path, text, &sel.kinds)?;
            if !sel.ignore.is_empty() {
                targets.retain(|t| !self.ignore_key(t).is_some_and(|k| sel.ignore.matches(&k)));
            }
            let src = Source::new(path, text);
            let src = &src;
            let sem = Semaphore::new(concurrency.max(1));
            let sem = &sem;
            let futures: Vec<_> = targets
                .iter()
                .map(|t| async move {
                    let _permit = sem.acquire().await.expect("semaphore is never closed");
                    self.resolve(ctx, src, t).await
                })
                .collect();
            Ok(join_all(futures).await)
        })
    }
}

/// Drive all `futures` concurrently on the current task until each is
/// complete, returning results in input order.
///
/// A stand-in for `futures::future::join_all` so the crate adds no
/// dependencies for this.
pub(crate) async fn join_all<F>(futures: Vec<F>) -> Vec<F::Output>
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kindset_defaults_contains_set() {
        let kinds = [
            KindSpec {
                id: "a",
                default_on: true,
                help: "",
            },
            KindSpec {
                id: "b",
                default_on: false,
                help: "",
            },
        ];
        let mut on = KindSet::defaults(&kinds);
        assert!(on.contains("a"));
        assert!(!on.contains("b"));
        assert!(!on.contains("unknown"));
        on.set("a", false);
        assert!(!on.contains("a"));
        on.set("b", true);
        assert!(on.contains("b"));
    }

    /// T-D17: `*` matches any run (incl. `/`, incl. empty), `?` one char,
    /// over the whole key.
    #[test]
    fn wildcard_matches_whole_key() {
        let table = [
            (
                "registry.example.com/legacy/*",
                "registry.example.com/legacy/a/b:1",
                true,
            ),
            ("*/legacy/*", "x/legacy/y", true),
            ("alpine:3.??", "alpine:3.20", true),
            ("alpine:3.??", "alpine:3.2", false),
            ("alpine", "alpine:3", false),
            ("alpine:3", "alpine", false),
            ("*", "", true),
            ("ä?", "äb", true),
            ("a*b*c", "a-b-x-c", true),
            ("a*b*c", "a-c-b", false),
            ("alpine:3.20*", "alpine:3.20@sha256:x", true),
        ];
        for (pattern, key, want) in table {
            let p = RefPatterns::new([pattern.to_string()]);
            assert_eq!(p.matches(key), want, "{pattern} vs {key}");
        }
        let none = RefPatterns::default();
        assert!(none.is_empty());
        assert!(!none.matches(""));
        let two = RefPatterns::new(["a".to_string(), "b*".to_string()]);
        assert!(!two.is_empty());
        assert!(two.matches("a") && two.matches("bc") && !two.matches("c"));
    }
}
