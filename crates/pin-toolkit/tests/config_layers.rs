//! Config layers end to end (spec §7.1, §7.2): repo file discovery,
//! precedence repo < env < CLI, fatal repo files.

mod support;

use std::collections::BTreeMap;
use std::path::Path;

use pin_toolkit::Process;
#[cfg(feature = "gitlab-ci")]
use pin_toolkit::cli::Mode;
use pin_toolkit::cli::parse;
use pin_toolkit::config::global::HostBinding;
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
    // Review-2 N1: from a subdirectory the root file is shown like walked
    // files, relative to the cwd.
    std::fs::create_dir_all(root.join("ci/deep")).unwrap();
    assert_eq!(
        load_in(&root, &root.join("ci/deep"), &["pin-toolkit"], &[]),
        Err(Fatal(
            "../../.pin-toolkit.toml: unsupported version 2 (expected 1)".into()
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
    // A templated image (skipped, no lookup): a code-0 row must show that
    // gitlab-ci ran (review-2 L5).
    write(&root, ".gitlab-ci.yml", "image: $CI_REGISTRY_IMAGE:1\n");
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
        } else {
            assert!(
                out.stdout.starts_with(".gitlab-ci.yml:1  ")
                    && out.stdout.contains("skipped (unparseable reference)"),
                "{argv:?} {vars:?}: gitlab-ci ran: {}",
                out.stdout
            );
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

// ---- the global layer (M5e', spec §7.1) ----------------------------------

/// `<root>/xdg/pin-toolkit/config.toml` written, `$XDG_CONFIG_HOME` value.
fn global_file(root: &Path, text: &str) -> String {
    write(root, "xdg/pin-toolkit/config.toml", text);
    root.join("xdg").to_str().unwrap().to_string()
}

#[test]
fn global_precedence_global_repo_env_cli() {
    let root = repo("global-precedence");
    let xdg = global_file(
        &root,
        "version = 1\njson = true\nconcurrency = 2\n\
         [hosts.\"gitlab.example.com\"]\nkind = \"gitlab\"\ntoken-env = \"EXAMPLE_TOKEN\"\n",
    );
    write(
        &root,
        ".pin-toolkit.toml",
        "version = 1\n[update]\nenabled = true\n",
    );
    let load = |argv: &[&str], extra: &[(&str, &str)]| {
        let mut vars = vec![("XDG_CONFIG_HOME", xdg.as_str())];
        vars.extend_from_slice(extra);
        load_in(&root, &root, argv, &vars).unwrap()
    };

    let none = load_in(&root, &root, &["pin-toolkit"], &[]).unwrap();
    assert_eq!((none.concurrency, none.json), (8, false), "no global file");
    assert!(none.hosts.is_empty() && none.update.enabled);

    // No key is allowed in both files (a repo `concurrency`/`hosts` is exit
    // 2), so global < repo is pinned by the fold test; here both apply.
    let e = load(&["pin-toolkit"], &[]);
    assert_eq!((e.concurrency, e.json), (2, true), "global over defaults");
    assert!(e.update.enabled, "the repo layer applies on the global one");
    assert_eq!(
        e.hosts,
        BTreeMap::from([(
            "gitlab.example.com".to_string(),
            HostBinding::Gitlab {
                token_env: "EXAMPLE_TOKEN".into()
            }
        )])
    );

    let env_over = [("PIN_TOOLKIT_CONCURRENCY", "3"), ("PIN_TOOLKIT_JSON", "0")];
    let e = load(&["pin-toolkit"], &env_over);
    assert_eq!((e.concurrency, e.json), (3, false), "env over global");
    let e = load(&["pin-toolkit", "--concurrency", "4"], &env_over);
    assert_eq!(e.concurrency, 4, "CLI over env over global");
}

#[test]
fn global_no_config_skips_global_and_repo() {
    let root = repo("global-no-config");
    let xdg = global_file(&root, "version = 2\n");
    write(&root, ".pin-toolkit.toml", "version = 2\n");
    let vars = [("XDG_CONFIG_HOME", xdg.as_str())];
    assert_eq!(
        load_in(&root, &root, &["pin-toolkit", "--no-config"], &vars),
        Ok(Effective::fold(&[]))
    );
    assert_eq!(
        load_in(
            &root,
            &root,
            &["pin-toolkit", "--no-config", "--config", "missing.toml"],
            &vars
        ),
        Ok(Effective::fold(&[])),
        "--no-config wins over --config"
    );
    // Without it the global file is read first, so its error comes first.
    assert_eq!(
        load_in(&root, &root, &["pin-toolkit"], &vars),
        Err(Fatal(
            "xdg/pin-toolkit/config.toml: unsupported version 2 (expected 1)".into()
        ))
    );

    let xdg = global_file(&root, "version = 1\nconcurrency = 2\n");
    let vars = [("XDG_CONFIG_HOME", xdg.as_str())];
    let eff = load_in(&root, &root, &["pin-toolkit", "--no-config"], &vars).unwrap();
    assert_eq!(eff.concurrency, 8, "a valid global file is skipped too");
}

#[test]
fn global_config_flag_keeps_global() {
    let root = repo("global-config-flag");
    let xdg = global_file(&root, "version = 1\nconcurrency = 2\n");
    let days = |n: u32| format!("version = 1\n[release_age]\nmin_days = {n}\n");
    write(&root, ".pin-toolkit.toml", &days(1));
    write(&root, "env.toml", &days(2));
    write(&root, "other.toml", &days(3));
    let load = |argv: &[&str], extra: &[(&str, &str)]| {
        let mut vars = vec![("XDG_CONFIG_HOME", xdg.as_str())];
        vars.extend_from_slice(extra);
        let e = load_in(&root, &root, argv, &vars).unwrap();
        (e.update.min_age_days, e.concurrency)
    };
    assert_eq!(load(&["pin-toolkit"], &[]), (1, 2), "root file + global");
    assert_eq!(
        load(&["pin-toolkit", "--config", "other.toml"], &[]),
        (3, 2),
        "--config replaces the repo file only"
    );
    assert_eq!(
        load(&["pin-toolkit"], &[("PIN_TOOLKIT_CONFIG", "env.toml")]),
        (2, 2),
        "PIN_TOOLKIT_CONFIG replaces the repo file only"
    );
    assert_eq!(
        load_in(
            &root,
            &root,
            &["pin-toolkit", "--config", "missing.toml"],
            &[("XDG_CONFIG_HOME", xdg.as_str())]
        ),
        Err(Fatal("config file not found: missing.toml".into())),
        "a given repo file must exist, whatever the global file says"
    );
}

#[test]
fn global_file_discovery() {
    let root = repo("global-discovery");
    let abs = |rel: &str| root.join(rel).to_str().unwrap().to_string();
    let concurrency = |vars: &[(&str, &str)]| {
        load_in(&root, &root, &["pin-toolkit"], vars)
            .unwrap()
            .concurrency
    };
    let (xdg, home) = (abs("xdg"), abs("home"));

    // Missing file = no layer (not an error), whichever variable names it.
    assert_eq!(concurrency(&[("XDG_CONFIG_HOME", &xdg)]), 8);
    assert_eq!(concurrency(&[("HOME", &home)]), 8);
    assert_eq!(concurrency(&[]), 8, "neither variable set");

    write(
        &root,
        "home/.config/pin-toolkit/config.toml",
        "version = 1\nconcurrency = 5\n",
    );
    assert_eq!(concurrency(&[("HOME", &home)]), 5, "$HOME/.config");
    write(
        &root,
        "xdg/pin-toolkit/config.toml",
        "version = 1\nconcurrency = 6\n",
    );
    let both = [("XDG_CONFIG_HOME", xdg.as_str()), ("HOME", home.as_str())];
    assert_eq!(concurrency(&both), 6, "$XDG_CONFIG_HOME over $HOME");
    let empty = [("XDG_CONFIG_HOME", ""), ("HOME", home.as_str())];
    assert_eq!(concurrency(&empty), 5, "empty XDG_CONFIG_HOME ignored");
    let relative = [("XDG_CONFIG_HOME", "xdg"), ("HOME", home.as_str())];
    assert_eq!(
        concurrency(&relative),
        5,
        "relative XDG_CONFIG_HOME ignored"
    );
}

#[tokio::test]
async fn global_bad_file_exits_2_naming_path() {
    let root = repo("global-bad");
    let xdg = global_file(&root, "version = 2\n");
    let vars = [("XDG_CONFIG_HOME", xdg.as_str())];
    write(&root, ".gitlab-ci.yml", "image: alpine:3.20\n");
    let out = run_in(&root, &["pin-toolkit"], &vars).await;
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        "pin-toolkit: xdg/pin-toolkit/config.toml: unsupported version 2 (expected 1)\n"
    );
    assert_eq!(out.stdout, "");
    assert_eq!(
        std::fs::read_to_string(root.join(".gitlab-ci.yml")).unwrap(),
        "image: alpine:3.20\n",
        "nothing is rewritten"
    );
    let out = run_in(&root, &["pin-toolkit", "plugins"], &vars).await;
    assert_eq!(
        (out.code, out.stdout.as_str()),
        (2, ""),
        "plugins loads the config too"
    );

    // Named relative to the cwd, like the repo file (review-2 N1).
    std::fs::create_dir_all(root.join("ci/deep")).unwrap();
    assert_eq!(
        load_in(&root, &root.join("ci/deep"), &["pin-toolkit"], &vars),
        Err(Fatal(
            "../../xdg/pin-toolkit/config.toml: unsupported version 2 (expected 1)".into()
        ))
    );

    // Unreadable (a directory in its place): the io error, with the path.
    std::fs::remove_file(root.join("xdg/pin-toolkit/config.toml")).unwrap();
    std::fs::create_dir(root.join("xdg/pin-toolkit/config.toml")).unwrap();
    let err = load_in(&root, &root, &["pin-toolkit"], &vars).expect_err("unreadable global file");
    assert!(
        err.0.starts_with("xdg/pin-toolkit/config.toml: ") && !err.0.contains("not found"),
        "{}",
        err.0
    );
}
