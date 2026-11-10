//! Config layers end to end (spec §7.1, §7.2): repo file discovery,
//! precedence repo < env < CLI, fatal repo files.

mod support;

use std::path::Path;

use pin_toolkit::Process;
use pin_toolkit::cli::parse;
use pin_toolkit::config::{Effective, Fatal, load};
use support::{args, env, repo, run_in, write};

/// `load` for `argv` in `cwd` (git root `root`) with the env map `vars`.
fn load_in(
    root: &Path,
    cwd: &Path,
    argv: &[&str],
    vars: &[(&str, &str)],
) -> Result<Effective, Fatal> {
    let p = Process {
        args: args(argv),
        env: env(vars),
        cwd: cwd.to_path_buf(),
    };
    let (cli, _) = parse(&p.args).unwrap();
    load(&p, root, &cli)
}

#[test]
fn precedence_repo_env_cli() {
    let root = repo("layers-precedence");
    write(
        &root,
        ".pin-toolkit.toml",
        "version = 1\n[update]\nenabled = true\n",
    );
    let env_off = [("PIN_TOOLKIT_UPDATE", "0")];
    let enabled = |argv: &[&str], vars: &[(&str, &str)]| {
        load_in(&root, &root, argv, vars).unwrap().update.enabled
    };
    assert!(
        enabled(&["pin-toolkit", "--update"], &env_off),
        "CLI over env"
    );
    assert!(!enabled(&["pin-toolkit"], &env_off), "env over repo");
    assert!(enabled(&["pin-toolkit"], &[]), "repo over defaults");
}

#[tokio::test]
async fn repo_hosts_exits_2() {
    let root = repo("layers-hosts");
    write(
        &root,
        ".pin-toolkit.toml",
        "version = 1\n[hosts.\"gitlab.example.com\"]\nkind = \"gitlab\"\n",
    );
    write(&root, ".gitlab-ci.yml", "image: alpine:3.20\n");
    let out = run_in(&root, &["pin-toolkit"], &[]).await;
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        "pin-toolkit: .pin-toolkit.toml: `hosts` is a global-only key; it is not allowed in a repository config\n"
    );
    assert_eq!(out.stdout, "");
    assert_eq!(
        std::fs::read_to_string(root.join(".gitlab-ci.yml")).unwrap(),
        "image: alpine:3.20\n"
    );
    let out = run_in(&root, &["pin-toolkit", "plugins"], &[]).await;
    assert_eq!(
        (out.code, out.stdout.as_str()),
        (2, ""),
        "plugins loads the config too"
    );
}

#[tokio::test]
async fn config_flag_and_env_path() {
    let root = repo("layers-config-path");
    let days = |n: u32| format!("version = 1\n[release_age]\nmin_days = {n}\n");
    write(&root, ".pin-toolkit.toml", &days(1));
    write(&root, "env.toml", &days(2));
    write(&root, "other.toml", &days(3));
    std::fs::create_dir_all(root.join("ci")).unwrap();
    let min_days = |cwd: &Path, argv: &[&str], vars: &[(&str, &str)]| {
        load_in(&root, cwd, argv, vars).unwrap().update.min_age_days
    };
    let env_file = [("PIN_TOOLKIT_CONFIG", "env.toml")];
    assert_eq!(min_days(&root, &["pin-toolkit"], &[]), 1, "root file");
    assert_eq!(
        min_days(&root.join("ci"), &["pin-toolkit"], &[]),
        1,
        "root file from a subdirectory"
    );
    assert_eq!(
        min_days(&root, &["pin-toolkit"], &env_file),
        2,
        "PIN_TOOLKIT_CONFIG over the root file"
    );
    assert_eq!(
        min_days(&root, &["pin-toolkit", "--config", "other.toml"], &env_file),
        3,
        "--config over PIN_TOOLKIT_CONFIG"
    );
    assert_eq!(
        min_days(
            &root.join("ci"),
            &["pin-toolkit", "--config", "../other.toml"],
            &[]
        ),
        3,
        "--config relative to the cwd"
    );

    let missing = Err(Fatal("config file not found: missing.toml".into()));
    assert_eq!(
        load_in(
            &root,
            &root,
            &["pin-toolkit", "--config", "missing.toml"],
            &[]
        ),
        missing
    );
    assert_eq!(
        load_in(
            &root,
            &root,
            &["pin-toolkit"],
            &[("PIN_TOOLKIT_CONFIG", "missing.toml")]
        ),
        missing
    );
    let out = run_in(&root, &["pin-toolkit", "--config", "missing.toml"], &[]).await;
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        "pin-toolkit: config file not found: missing.toml\n"
    );
    assert_eq!(out.stdout, "");
}

#[test]
fn no_config_skips_repo_file() {
    let root = repo("layers-no-config");
    write(&root, ".pin-toolkit.toml", "version = 2\n");
    assert_eq!(
        load_in(&root, &root, &["pin-toolkit", "--no-config"], &[]),
        Ok(Effective::fold(&[]))
    );
    assert_eq!(
        load_in(
            &root,
            &root,
            &["pin-toolkit", "--no-config", "--config", "missing.toml"],
            &[]
        ),
        Ok(Effective::fold(&[])),
        "--no-config wins over --config"
    );
    assert_eq!(
        load_in(&root, &root, &["pin-toolkit"], &[]),
        Err(Fatal(
            ".pin-toolkit.toml: unsupported version 2 (expected 1)".into()
        ))
    );
}
