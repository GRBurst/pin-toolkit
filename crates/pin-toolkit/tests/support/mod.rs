//! Shared helpers of the pin-toolkit integration tests: scratch repos under
//! `$CARGO_MANIFEST_DIR/.tmp/` (always with a `.git/` directory, so the
//! git-root walk-up never reaches this repository) and `execute` calls with
//! an explicit env map (no machine credentials reach a test).

#![allow(dead_code)] // every test binary uses a different subset

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use pin_core::run::ResolverEnv;
use pin_toolkit::{Output, Process, execute};

/// A fresh scratch repo `.tmp/{name}-{pid}/` with an empty `.git/`.
pub fn repo(name: &str) -> PathBuf {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(".tmp/{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

/// Write `text` to `root/rel` (parents created).
pub fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// `argv` as `OsString`s.
pub fn args(argv: &[&str]) -> Vec<OsString> {
    argv.iter().map(OsString::from).collect()
}

/// An env map from pairs.
pub fn env(vars: &[(&str, &str)]) -> BTreeMap<String, String> {
    vars.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// `execute(argv)` in `cwd` with the env map `vars`, resolvers untouched.
pub async fn run_in(cwd: &Path, argv: &[&str], vars: &[(&str, &str)]) -> Output {
    run_tuned(cwd, argv, vars, |_| {}).await
}

/// [`run_in`] with `tune` applied to the resolver env (fakes).
pub async fn run_tuned(
    cwd: &Path,
    argv: &[&str],
    vars: &[(&str, &str)],
    tune: impl FnOnce(&mut ResolverEnv),
) -> Output {
    execute(
        Process {
            args: args(argv),
            env: env(vars),
            cwd: cwd.to_path_buf(),
        },
        tune,
    )
    .await
}

/// Resolvers pointed at fakes: registry hosts `docker.io` and `ghcr.io` →
/// `registry`, GitLab `gitlab.com` → `gitlab` (API base).
pub fn fakes(registry: String, gitlab: String) -> impl FnOnce(&mut ResolverEnv) {
    move |env| {
        for host in ["docker.io", "ghcr.io"] {
            env.base_urls
                .registry
                .insert(host.to_string(), registry.clone());
        }
        env.base_urls
            .gitlab
            .insert("gitlab.com".to_string(), gitlab);
    }
}
