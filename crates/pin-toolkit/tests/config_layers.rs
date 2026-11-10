//! Config layers end to end (spec §7.1, §7.2): repo file discovery,
//! precedence repo < env < CLI, fatal repo files.

mod support;

use std::path::Path;

use pin_toolkit::Process;
#[cfg(feature = "gitlab-ci")]
use pin_toolkit::cli::Mode;
use pin_toolkit::cli::parse;
use pin_toolkit::config::{Effective, Fatal, load};
#[cfg(feature = "gitlab-ci")]
use pin_toolkit::enable::enable;
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

#[cfg(feature = "gitlab-ci")]
#[tokio::test]
async fn run_false_disables_but_subcommand_runs() {
    let root = repo("enable-run-false");
    write(
        &root,
        ".pin-toolkit.toml",
        "version = 1\n[plugins.gitlab-ci]\nrun = false\n",
    );
    let out = run_in(&root, &["pin-toolkit", "plugins"], &[]).await;
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout.lines().next(), Some("gitlab-ci: disabled"));

    let eff = load_in(&root, &root, &["pin-toolkit"], &[]).unwrap();
    assert_eq!(enable(&eff, &Mode::All).unwrap().ids(), Vec::<&str>::new());
    assert_eq!(
        enable(&eff, &Mode::Plugin("gitlab-ci")).unwrap().ids(),
        ["gitlab-ci"]
    );

    let out = run_in(
        &root,
        &["pin-toolkit", "plugins", "--kind", "gitlab-ci:include=off"],
        &[],
    )
    .await;
    let include = pin_gitlab_ci::KINDS
        .iter()
        .find(|k| k.id == "include")
        .unwrap();
    assert!(
        out.stdout
            .contains(&format!("\n  include: off — {}\n", include.help)),
        "kind state from the config: {}",
        out.stdout
    );

    let eff = load_in(&root, &root, &["pin-toolkit", "--no-config"], &[]).unwrap();
    assert_eq!(enable(&eff, &Mode::All).unwrap().ids(), ["gitlab-ci"]);
    write(
        &root,
        ".pin-toolkit.toml",
        "version = 1\n[plugins.gitlab-ci]\nfiles = [\"[\"]\n",
    );
    // Review-2 L2: the repo parser rejects the glob (with the path) ...
    let err = load_in(&root, &root, &["pin-toolkit"], &[]).expect_err("invalid glob is fatal");
    assert!(
        err.0
            .starts_with(".pin-toolkit.toml: invalid glob `[` in plugins.gitlab-ci.files: "),
        "{}",
        err.0
    );
    // ... and enable still builds every glob it uses.
    let mut eff = Effective::fold(&[]);
    eff.plugins.entry("gitlab-ci".into()).or_default().files = Some(vec!["[".into()]);
    let err = enable(&eff, &Mode::All)
        .err()
        .expect("invalid glob is fatal");
    assert!(
        err.0
            .starts_with("invalid glob `[` in plugins.gitlab-ci.files: "),
        "{}",
        err.0
    );
}

#[cfg(feature = "gitlab-ci")]
#[tokio::test]
async fn only_and_subcommand_rules() {
    let root = repo("enable-only");
    let known = "(known: gitlab-ci, helm, github-actions)";
    let conflict = "pin-toolkit: --only/--skip cannot be combined with a plugin subcommand\n";
    // (argv, env, code, stderr)
    type Row<'a> = (&'a [&'a str], &'a [(&'a str, &'a str)], u8, String);
    let rows: Vec<Row> = vec![
        (
            &["pin-toolkit", "--only", "helm"],
            &[],
            2,
            "pin-toolkit: plugin helm not in this build\n".into(),
        ),
        (
            &["pin-toolkit", "--only", "nope"],
            &[],
            2,
            format!("pin-toolkit: unknown plugin `nope` {known}\n"),
        ),
        (
            &["pin-toolkit", "--skip", "nope"],
            &[],
            2,
            format!("pin-toolkit: unknown plugin `nope` {known}\n"),
        ),
        (&["pin-toolkit", "--skip", "helm"], &[], 0, String::new()),
        (
            &["pin-toolkit", "gitlab-ci", "--skip", "x"],
            &[],
            2,
            conflict.into(),
        ),
        (
            &["pin-toolkit", "--only", "gitlab-ci", "gitlab-ci"],
            &[],
            2,
            conflict.into(),
        ),
        (
            &["pin-toolkit", "helm"],
            &[],
            2,
            "pin-toolkit: plugin helm not in this build\n".into(),
        ),
        // PR-L6: env ONLY/SKIP are ignored in subcommand/alias mode ...
        (
            &["pin-toolkit", "gitlab-ci", "--dry-run"],
            &[("PIN_TOOLKIT_SKIP", "gitlab-ci")],
            0,
            String::new(),
        ),
        (
            &["pin-ci"],
            &[("PIN_TOOLKIT_ONLY", "helm")],
            0,
            String::new(),
        ),
        // ... and apply without one.
        (
            &["pin-toolkit"],
            &[("PIN_TOOLKIT_ONLY", "helm")],
            2,
            "pin-toolkit: plugin helm not in this build\n".into(),
        ),
    ];
    for (argv, vars, code, stderr) in rows {
        let out = run_in(&root, argv, vars).await;
        assert_eq!(
            (out.code, out.stderr.as_str()),
            (code, stderr.as_str()),
            "{argv:?} {vars:?}"
        );
        if code == 2 {
            assert_eq!(out.stdout, "", "{argv:?}");
        }
    }
    let eff = load_in(
        &root,
        &root,
        &["pin-toolkit"],
        &[("PIN_TOOLKIT_SKIP", "gitlab-ci")],
    )
    .unwrap();
    assert_eq!(
        enable(&eff, &Mode::Plugin("gitlab-ci")).unwrap().ids(),
        ["gitlab-ci"]
    );
    assert_eq!(enable(&eff, &Mode::All).unwrap().ids(), Vec::<&str>::new());
}

#[tokio::test]
async fn not_built_plugin_in_config_is_a_notice() {
    let root = repo("enable-notice");
    let notice = "pin-toolkit: skipped (plugin helm not in this build)\n";
    for (table, count) in [
        ("run = true\n", 1),
        ("kinds = { values-image = false }\n", 1),
        ("run = false\n", 0),
    ] {
        write(
            &root,
            ".pin-toolkit.toml",
            &format!("version = 1\n[plugins.helm]\n{table}"),
        );
        let out = run_in(&root, &["pin-toolkit"], &[]).await;
        assert_eq!(out.code, 0, "{table}: {}", out.stderr);
        assert_eq!(
            out.stderr.matches(notice).count(),
            count,
            "{table}: {}",
            out.stderr
        );
        assert_eq!(
            out.stderr.len(),
            notice.len() * count,
            "{table}: {}",
            out.stderr
        );
    }
}

#[cfg(not(feature = "gitlab-ci"))]
#[tokio::test]
async fn gitlab_ci_subcommand_not_in_build() {
    let root = repo("enable-nodefault");
    let out = run_in(&root, &["pin-toolkit", "gitlab-ci"], &[]).await;
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        "pin-toolkit: plugin gitlab-ci not in this build\n"
    );
    assert_eq!(out.stdout, "");
}
