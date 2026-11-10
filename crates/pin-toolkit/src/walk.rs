//! Inputs of a run (spec §6.3): the git root.

use std::path::{Path, PathBuf};

/// The first ancestor of `cwd` (incl.) that contains a `.git` entry.
pub fn git_root(cwd: &Path) -> Option<PathBuf> {
    git_root_with(cwd, |p| p.exists())
}

/// [`git_root`] with the existence check injected (tests).
fn git_root_with(cwd: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|a| exists(&a.join(".git")))
        .map(Path::to_path_buf)
}
