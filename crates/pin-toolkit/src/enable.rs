//! Which plugins run, with which kinds, ignore patterns and file globs
//! (spec §6.5, §6.6, §8.2/O1; T-D11, T-D28).

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use pin_core::plugin::{DynPlugin, KindSet, RefPatterns, Selection};

use crate::cli::Mode;
use crate::config::repo::unknown_plugin;
use crate::config::{Effective, Fatal};
use crate::known::{compiled, compiled_plugin, known};

/// The plugins of one run, in `compiled()` order, and the notices to print
/// (stderr, exit code unaffected).
pub struct Plan {
    pub plugins: Vec<(&'static dyn DynPlugin, Selection, GlobSet)>,
    pub notices: Vec<String>,
}

impl Plan {
    /// The enabled plugin ids, in order.
    pub fn ids(&self) -> Vec<&'static str> {
        self.plugins.iter().map(|(p, ..)| p.id()).collect()
    }
}

/// `plugin {id} not in this build` (exit 2).
fn not_in_build(id: &str) -> Fatal {
    Fatal(format!("plugin {id} not in this build"))
}

/// The plugins `mode` runs. A subcommand/alias runs its plugin (it must be
/// compiled in) and never consults `only`/`skip`, which in that mode can
/// come only from the env (PR-L6). Otherwise a compiled plugin runs unless
/// `only` leaves it out, `skip` names it or `plugins.<id>.run = false`;
/// `only`/`skip` ids must be known, and an `only` id must be compiled in
/// (T-D11). In `Mode::All`, a config table for a known plugin that is not
/// compiled in (unless `run = false`) gives a notice, never an error.
pub fn enable(eff: &Effective, mode: &Mode) -> Result<Plan, Fatal> {
    let mut notices = Vec::new();
    let ids: Vec<&'static str> = match *mode {
        Mode::Plugin(id) => {
            compiled_plugin(id).ok_or_else(|| not_in_build(id))?;
            vec![id]
        }
        Mode::All | Mode::Plugins => {
            let only = eff.only.as_deref();
            for id in only.into_iter().flatten().chain(&eff.skip) {
                if known(id).is_none() {
                    return Err(Fatal(unknown_plugin(id)));
                }
            }
            if let Some(id) = only
                .into_iter()
                .flatten()
                .find(|id| compiled_plugin(id).is_none())
            {
                return Err(not_in_build(id));
            }
            if *mode == Mode::All {
                for (id, pl) in &eff.plugins {
                    if compiled_plugin(id).is_none() && pl.run != Some(false) {
                        notices.push(format!("skipped (plugin {id} not in this build)"));
                    }
                }
            }
            compiled()
                .iter()
                .map(|p| p.id())
                .filter(|id| only.is_none_or(|o| o.iter().any(|x| x == id)))
                .filter(|id| !eff.skip.iter().any(|x| x == id))
                .filter(|id| eff.plugins.get(*id).and_then(|p| p.run) != Some(false))
                .collect()
        }
    };
    let mut plugins = Vec::with_capacity(ids.len());
    for id in ids {
        let p = compiled_plugin(id).expect("enabled plugins are compiled");
        plugins.push((p, selection(eff, p), globs(eff, p)?));
    }
    Ok(Plan { plugins, notices })
}

/// `p`'s kinds (its defaults, then `plugins.<id>.kinds` for its compiled
/// kinds; known kinds it does not have yet, e.g. `helm-oci`, are ignored)
/// and its `ignore` patterns.
pub fn selection(eff: &Effective, p: &dyn DynPlugin) -> Selection {
    let mut kinds = KindSet::defaults(p.kinds());
    let pl = eff.plugins.get(p.id());
    for (k, on) in pl.iter().flat_map(|pl| &pl.kinds) {
        if let Some(spec) = p.kinds().iter().find(|s| s.id == k) {
            kinds.set(spec.id, *on);
        }
    }
    Selection {
        kinds,
        ignore: RefPatterns::new(pl.iter().flat_map(|pl| pl.ignore.iter().cloned())),
    }
}

/// `plugins.<id>.files`, else `p`'s default globs, matched against the
/// path relative to the git root (`*` does not cross `/`).
fn globs(eff: &Effective, p: &dyn DynPlugin) -> Result<GlobSet, Fatal> {
    let configured = eff.plugins.get(p.id()).and_then(|pl| pl.files.clone());
    let files =
        configured.unwrap_or_else(|| p.default_files().iter().map(|g| g.to_string()).collect());
    let mut set = GlobSetBuilder::new();
    for g in &files {
        let glob = GlobBuilder::new(g)
            .literal_separator(true)
            .build()
            .map_err(|e| {
                Fatal(format!(
                    "invalid glob `{g}` in plugins.{}.files: {}",
                    p.id(),
                    e.kind()
                ))
            })?;
        set.add(glob);
    }
    set.build()
        .map_err(|e| Fatal(format!("plugins.{}.files: {}", p.id(), e.kind())))
}
