//! The CLI front-end through `execute` (spec §6.6).

mod support;

use std::path::Path;

use support::{repo, run_in, write};

#[cfg(feature = "gitlab-ci")]
#[tokio::test]
async fn plugins_lists_compiled_and_known() {
    let root = repo("cli-plugins");
    let out = run_in(&root, &["pin-toolkit", "plugins"], &[]).await;
    let mut expected = String::from("gitlab-ci: enabled\n");
    for k in pin_gitlab_ci::KINDS {
        expected.push_str(&format!("  {}: on — {}\n", k.id, k.help));
    }
    expected.push_str("helm: not in this build\ngithub-actions: not in this build\n");
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, expected);
    assert_eq!(out.stderr, "");
}

/// M5d final L3: `plugins` explains a run whose `only` names a plugin that
/// is not in this build instead of failing like that run: exit 0, the
/// listing (gitlab-ci disabled by `only`), a note on stderr. An unknown id
/// stays an error.
#[cfg(feature = "gitlab-ci")]
#[tokio::test]
async fn plugins_notes_an_unbuilt_only_plugin() {
    let root = repo("cli-plugins-only-unbuilt");
    let mut expected = String::from("gitlab-ci: disabled\n");
    for k in pin_gitlab_ci::KINDS {
        expected.push_str(&format!("  {}: on — {}\n", k.id, k.help));
    }
    expected.push_str("helm: not in this build\ngithub-actions: not in this build\n");
    let note = "pin-toolkit: note: plugin helm not in this build (named by --only/PIN_TOOLKIT_ONLY; a run exits 2)\n";
    type Row<'a> = (&'a [&'a str], &'a [(&'a str, &'a str)]);
    let rows: [Row; 3] = [
        (&["pin-toolkit", "--only", "helm", "plugins"], &[]),
        (&["pin-toolkit", "plugins", "--only", "helm"], &[]),
        (&["pin-toolkit", "plugins"], &[("PIN_TOOLKIT_ONLY", "helm")]),
    ];
    for (argv, vars) in rows {
        let out = run_in(&root, argv, vars).await;
        assert_eq!(
            (out.code, out.stdout.as_str(), out.stderr.as_str()),
            (0, expected.as_str(), note),
            "{argv:?} {vars:?}"
        );
    }
    let out = run_in(&root, &["pin-toolkit", "--only", "nope", "plugins"], &[]).await;
    assert_eq!(out.code, 2, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "");
}

#[cfg(not(feature = "gitlab-ci"))]
#[tokio::test]
async fn plugins_without_gitlab_ci() {
    let root = repo("cli-plugins-nodefault");
    let out = run_in(&root, &["pin-toolkit", "plugins"], &[]).await;
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout.lines().next(),
        Some("gitlab-ci: not in this build")
    );
}

#[tokio::test]
async fn usage_error_exits_2() {
    let root = repo("cli-usage");
    let out = run_in(&root, &["pin-toolkit", "--bogus"], &[]).await;
    assert_eq!(out.code, 2);
    assert!(out.stderr.starts_with("error:"), "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "");

    let out = run_in(&root, &["pin-toolkit", "--help"], &[]).await;
    assert_eq!(out.code, 0);
    assert!(
        out.stdout.contains("Usage: pin-toolkit"),
        "stdout: {}",
        out.stdout
    );
    // M5d final N3: no Helm in the about text until the helm plugin (M7).
    assert!(
        out.stdout.starts_with("Pin CI references to immutable ids")
            && !out.stdout.contains("Helm"),
        "stdout: {}",
        out.stdout
    );
    // M5e' M4: the primary host's precedence names `GITLAB_URI`.
    assert!(
        out.stdout
            .contains("(default: GITLAB_URI, else CI_SERVER_HOST, else gitlab.com)"),
        "stdout: {}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("Do not read the global or repo config file"),
        "stdout: {}",
        out.stdout
    );
    assert_eq!(out.stderr, "");

    let out = run_in(&root, &["pin-toolkit", "--version"], &[]).await;
    assert_eq!((out.code, out.stdout.as_str()), (0, "pin-toolkit 0.1.0\n"));
}

// ---- `config show` (M5e', design S5) ---------------------------------------

/// The `plugins.gitlab-ci.*` lines of a default config (what a build with
/// the plugin prints; empty without it).
fn gitlab_ci_defaults() -> &'static str {
    if cfg!(feature = "gitlab-ci") {
        concat!(
            "plugins.gitlab-ci.run = true\n",
            "plugins.gitlab-ci.files = [\".gitlab-ci.yml\", \"**/*.gitlab-ci.yml\", \".gitlab/ci/**/*.yml\"]\n",
            "plugins.gitlab-ci.ignore = []\n",
            "plugins.gitlab-ci.kinds.image = true\n",
            "plugins.gitlab-ci.kinds.service = true\n",
            "plugins.gitlab-ci.kinds.component = true\n",
            "plugins.gitlab-ci.kinds.include = true\n",
            "plugins.gitlab-ci.gitlab-hosts = []\n",
        )
    } else {
        ""
    }
}

/// No config file at all: the defaults, `none` for both files, the host
/// `gitlab.com`; nothing on stderr.
#[tokio::test]
async fn config_show_prints_defaults() {
    let root = repo("cli-config-show-defaults");
    let out = run_in(&root, &["pin-toolkit", "config", "show"], &[]).await;
    let want = format!(
        "config.global-file = none\n\
         config.repo-file = none\n\
         update.enabled = false\n\
         update.major = false\n\
         release-age.min-days = 0\n\
         release-age.unknown = allow\n\
         concurrency = 8\n\
         json = false\n\
         gitlab-host = gitlab.com\n\
         {}",
        gitlab_ci_defaults()
    );
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (0, want.as_str(), "")
    );
}

/// The global file's `json`/`concurrency`/`[hosts]` and the repo file's
/// update/release-age/plugin keys; the bound token's VALUE is never printed
/// (only the env var NAME).
#[tokio::test]
async fn config_show_prints_global_and_repo_values() {
    let root = repo("cli-config-show-values");
    write(
        &root,
        "xdg/pin-toolkit/config.toml",
        "version = 1\njson = true\nconcurrency = 3\n\
         [hosts.\"gitlab.example.com\"]\nkind = \"gitlab\"\ntoken-env = \"GITLAB_EXAMPLE_TOKEN\"\n\
         [hosts.\"charts.example.com\"]\nkind = \"helm-repo\"\n\
         username-env = \"CHARTS_USER\"\npassword-env = \"CHARTS_PASSWORD\"\n\
         [hosts.\"registry.example.com\"]\nkind = \"oci\"\nca-file = \"/etc/ssl/corp-ca.pem\"\n",
    );
    write(
        &root,
        ".pin-toolkit.toml",
        "version = 1\n[update]\nenabled = true\nmajor = true\n\
         [release_age]\nmin_days = 7\nunknown = \"block\"\n\
         [plugins.helm]\nrun = false\n",
    );
    let xdg = root.join("xdg").display().to_string();
    let vars = [
        ("XDG_CONFIG_HOME", xdg.as_str()),
        ("GITLAB_EXAMPLE_TOKEN", "glpat-SECRET-VALUE"),
        ("CHARTS_PASSWORD", "hunter2-SECRET-VALUE"),
        ("GITLAB_TOKEN", "glpat-PRIMARY-SECRET"),
        ("CI_JOB_TOKEN", "job-SECRET"),
    ];
    let out = run_in(&root, &["pin-toolkit", "config", "show"], &vars).await;
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "");
    let mut want = String::from(
        "config.global-file = xdg/pin-toolkit/config.toml\n\
         config.repo-file = .pin-toolkit.toml\n\
         update.enabled = true\n\
         update.major = true\n\
         release-age.min-days = 7\n\
         release-age.unknown = block\n\
         concurrency = 3\n\
         json = true\n\
         gitlab-host = gitlab.com\n",
    );
    want.push_str(gitlab_ci_defaults());
    want.push_str(
        "plugins.helm.run = false\n\
         hosts.charts.example.com.kind = helm-repo\n\
         hosts.charts.example.com.username-env = CHARTS_USER\n\
         hosts.charts.example.com.password-env = CHARTS_PASSWORD\n\
         hosts.gitlab.example.com.kind = gitlab\n\
         hosts.gitlab.example.com.token-env = GITLAB_EXAMPLE_TOKEN\n\
         hosts.registry.example.com.kind = oci\n\
         hosts.registry.example.com.ca-file = /etc/ssl/corp-ca.pem\n",
    );
    assert_eq!(out.stdout, want);
    for secret in ["SECRET-VALUE", "PRIMARY-SECRET", "job-SECRET"] {
        assert!(
            !out.stdout.contains(secret) && !out.stderr.contains(secret),
            "{secret} leaked"
        );
    }
}

/// The `gitlab-host` line of `pin-toolkit <argv>` in `root`.
async fn gitlab_host_of(root: &Path, argv: &[&str], vars: &[(&str, &str)]) -> String {
    let argv: Vec<&str> = ["pin-toolkit"].iter().chain(argv).copied().collect();
    let out = run_in(root, &argv, vars).await;
    assert_eq!(out.code, 0, "{argv:?} stderr: {}", out.stderr);
    out.stdout
        .lines()
        .find_map(|l| l.strip_prefix("gitlab-host = "))
        .map(String::from)
        .unwrap()
}

/// The primary host as the run would use it: `--gitlab-host` over
/// `GITLAB_URI` over `CI_SERVER_HOST` over `gitlab.com`; `--no-config`
/// shows no file.
#[tokio::test]
async fn config_show_prints_the_effective_primary_host() {
    let root = repo("cli-config-show-host");
    write(&root, ".pin-toolkit.toml", "version = 1\n");
    let host = |argv: &'static [&'static str], vars: &'static [(&'static str, &'static str)]| {
        gitlab_host_of(&root, argv, vars)
    };
    const BOTH: &[(&str, &str)] = &[
        ("GITLAB_URI", "https://uri.example.com/"),
        ("CI_SERVER_HOST", "ci.example.com"),
    ];
    assert_eq!(host(&["config", "show"], &[]).await, "gitlab.com");
    assert_eq!(
        host(&["config", "show"], &[("CI_SERVER_HOST", "ci.example.com")]).await,
        "ci.example.com"
    );
    assert_eq!(host(&["config", "show"], BOTH).await, "uri.example.com");
    assert_eq!(
        host(
            &["config", "show", "--gitlab-host", "cli.example.com"],
            BOTH
        )
        .await,
        "cli.example.com"
    );
    assert_eq!(
        host(
            &["config", "show"],
            &[("PIN_TOOLKIT_GITLAB_HOST", "env.example.com")]
        )
        .await,
        "env.example.com"
    );

    let out = run_in(
        &root,
        &["pin-toolkit", "--no-config", "config", "show"],
        &[],
    )
    .await;
    assert!(
        out.stdout
            .starts_with("config.global-file = none\nconfig.repo-file = none\n"),
        "{}",
        out.stdout
    );
}

/// A bad config in any layer or an invalid host env exits 2 with nothing on
/// stdout, like any run (`config show` loads and validates first).
#[tokio::test]
async fn config_show_prints_nothing_on_bad_config() {
    let root = repo("cli-config-show-bad");
    let argv = ["pin-toolkit", "config", "show"];

    let out = run_in(&root, &argv, &[("CI_SERVER_HOST", "bad host!")]).await;
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (
            2,
            "",
            "pin-toolkit: CI_SERVER_HOST: invalid GitLab host `bad host!`\n"
        )
    );
    let out = run_in(&root, &argv, &[("GITLAB_URI", "https://bad_host/")]).await;
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (
            2,
            "",
            "pin-toolkit: GITLAB_URI: invalid GitLab host `bad_host`\n"
        )
    );

    write(&root, ".pin-toolkit.toml", "version = 2\n");
    let out = run_in(&root, &argv, &[]).await;
    assert_eq!((out.code, out.stdout.as_str()), (2, ""));
    assert!(
        out.stderr.starts_with("pin-toolkit: .pin-toolkit.toml: "),
        "stderr: {}",
        out.stderr
    );

    write(&root, ".pin-toolkit.toml", "version = 1\n");
    write(&root, "xdg/pin-toolkit/config.toml", "version = 2\n");
    let xdg = root.join("xdg").display().to_string();
    let out = run_in(&root, &argv, &[("XDG_CONFIG_HOME", xdg.as_str())]).await;
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (
            2,
            "",
            "pin-toolkit: xdg/pin-toolkit/config.toml: unsupported version 2 (expected 1)\n"
        )
    );

    let out = run_in(&root, &argv, &[("PIN_TOOLKIT_CONCURRENCY", "x")]).await;
    assert_eq!((out.code, out.stdout.as_str()), (2, ""));
}

/// `--only`/`--skip` do not combine with the subcommand (exit 2); the env
/// `PIN_TOOLKIT_ONLY`/`SKIP` are ignored, even an unknown plugin id.
#[tokio::test]
async fn config_show_prints_rejects_only_skip_ignores_their_env() {
    let root = repo("cli-config-show-only");
    for flag in ["--only", "--skip"] {
        for argv in [
            ["pin-toolkit", "config", "show", flag, "gitlab-ci"],
            ["pin-toolkit", flag, "gitlab-ci", "config", "show"],
        ] {
            let out = run_in(&root, &argv, &[]).await;
            assert_eq!(
                (out.code, out.stdout.as_str(), out.stderr.as_str()),
                (
                    2,
                    "",
                    "pin-toolkit: --only/--skip cannot be combined with `config show`\n"
                ),
                "{argv:?}"
            );
        }
    }
    let plain = run_in(&root, &["pin-toolkit", "config", "show"], &[]).await;
    let vars = [
        ("PIN_TOOLKIT_ONLY", "nope"),
        ("PIN_TOOLKIT_SKIP", "gitlab-ci"),
    ];
    let out = run_in(&root, &["pin-toolkit", "config", "show"], &vars).await;
    assert_eq!((out.code, out.stderr.as_str()), (0, ""));
    assert_eq!(out.stdout, plain.stdout, "env ONLY/SKIP change nothing");
}

/// `config show` walks no file and resolves nothing: a scannable file is
/// left alone and no report is printed (no resolver is built, so no
/// network).
#[tokio::test]
async fn config_show_prints_config_only_no_run() {
    let root = repo("cli-config-show-norun");
    write(&root, ".gitlab-ci.yml", "image: alpine:3.20\n");
    let out = run_in(&root, &["pin-toolkit", "config", "show", "--update"], &[]).await;
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("update.enabled = true\n"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("pinned:"), "no report: {}", out.stdout);
    assert_eq!(
        std::fs::read_to_string(root.join(".gitlab-ci.yml")).unwrap(),
        "image: alpine:3.20\n"
    );
}
