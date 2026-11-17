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
    let want = format!("pin-toolkit {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!((out.code, out.stdout.as_str()), (0, want.as_str()));
}

// ---- `config show` (M5e', design S5) ---------------------------------------

/// Every credential-ish variable a run reads (the registry and GitLab
/// ones, and the ones the host bindings below name), all holding a secret.
/// `config show` prints variable NAMES only, never these values.
const SECRETS: &[(&str, &str)] = &[
    ("GITLAB_TOKEN", "glpat-SECRET-VALUE-1"),
    ("CI_JOB_TOKEN", "job-SECRET-VALUE-2"),
    ("DOCKERHUB_USER", "user-SECRET-VALUE-3"),
    ("DOCKERHUB_TOKEN", "dh-SECRET-VALUE-4"),
    ("GITHUB_TOKEN", "ghp-SECRET-VALUE-5"),
    ("GITLAB_EXAMPLE_TOKEN", "glpat-SECRET-VALUE-6"),
    ("CHARTS_USER", "charts-SECRET-VALUE-7"),
    ("CHARTS_PASSWORD", "hunter2-SECRET-VALUE-8"),
];

/// [`run_in`] with [`SECRETS`] added to `vars`; fails if any secret value
/// reaches stdout or stderr, whatever else the test asserts.
async fn run_config(cwd: &Path, argv: &[&str], vars: &[(&str, &str)]) -> pin_toolkit::Output {
    let all: Vec<(&str, &str)> = vars.iter().chain(SECRETS).copied().collect();
    let out = run_in(cwd, argv, &all).await;
    assert!(
        !out.stdout.contains("SECRET-VALUE") && !out.stderr.contains("SECRET-VALUE"),
        "a secret value leaked: {argv:?}\nstdout: {}\nstderr: {}",
        out.stdout,
        out.stderr
    );
    out
}

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
    let out = run_config(&root, &["pin-toolkit", "config", "show"], &[]).await;
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
    let vars = [("XDG_CONFIG_HOME", xdg.as_str())];
    let out = run_config(&root, &["pin-toolkit", "config", "show"], &vars).await;
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
}

/// The `gitlab-host` line of `pin-toolkit <argv>` in `root`.
async fn gitlab_host_of(root: &Path, argv: &[&str], vars: &[(&str, &str)]) -> String {
    let argv: Vec<&str> = ["pin-toolkit"].iter().chain(argv).copied().collect();
    let out = run_config(root, &argv, vars).await;
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

    let out = run_config(
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

    let out = run_config(&root, &argv, &[("CI_SERVER_HOST", "bad host!")]).await;
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (
            2,
            "",
            "pin-toolkit: CI_SERVER_HOST: invalid GitLab host `bad host!`\n"
        )
    );
    let out = run_config(&root, &argv, &[("GITLAB_URI", "https://bad_host/")]).await;
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (
            2,
            "",
            "pin-toolkit: GITLAB_URI: invalid GitLab host `bad_host`\n"
        )
    );

    write(&root, ".pin-toolkit.toml", "version = 2\n");
    let out = run_config(&root, &argv, &[]).await;
    assert_eq!((out.code, out.stdout.as_str()), (2, ""));
    assert!(
        out.stderr.starts_with("pin-toolkit: .pin-toolkit.toml: "),
        "stderr: {}",
        out.stderr
    );

    write(&root, ".pin-toolkit.toml", "version = 1\n");
    write(&root, "xdg/pin-toolkit/config.toml", "version = 2\n");
    let xdg = root.join("xdg").display().to_string();
    let out = run_config(&root, &argv, &[("XDG_CONFIG_HOME", xdg.as_str())]).await;
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (
            2,
            "",
            "pin-toolkit: xdg/pin-toolkit/config.toml: unsupported version 2 (expected 1)\n"
        )
    );

    let out = run_config(&root, &argv, &[("PIN_TOOLKIT_CONCURRENCY", "x")]).await;
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
            let out = run_config(&root, &argv, &[]).await;
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
    let plain = run_config(&root, &["pin-toolkit", "config", "show"], &[]).await;
    let vars = [
        ("PIN_TOOLKIT_ONLY", "nope"),
        ("PIN_TOOLKIT_SKIP", "gitlab-ci"),
    ];
    let out = run_config(&root, &["pin-toolkit", "config", "show"], &vars).await;
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
    let out = run_config(&root, &["pin-toolkit", "config", "show", "--update"], &[]).await;
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

/// `./config` (and a `config` after a path) is a path, not the subcommand:
/// a normal run, whether or not the file exists, never prints the
/// configuration.
#[tokio::test]
async fn config_show_is_not_run_for_a_config_path() {
    let root = repo("cli-config-show-path");
    for argv in [
        &["pin-toolkit", "./config"][..],
        &["pin-toolkit", "./config", "config", "show"],
    ] {
        // No such file.
        let out = run_config(&root, argv, &[]).await;
        assert!(
            !out.stdout.contains("config.global-file") && !out.stdout.contains("config.repo-file"),
            "{argv:?}: {}",
            out.stdout
        );
        assert_eq!(out.code, 2, "{argv:?}: a missing path is a usage error");
        assert!(out.stderr.contains("./config"), "{argv:?}: {}", out.stderr);
    }
    // A file named `config` with nothing to pin: a normal run, no listing.
    write(&root, "config", "image: alpine\n");
    let out = run_config(&root, &["pin-toolkit", "./config"], &[]).await;
    assert!(
        !out.stdout.contains("config.global-file") && !out.stdout.contains("config.repo-file"),
        "{}",
        out.stdout
    );
}

// ---- `config show --origin` (M5e', design S5) ------------------------------

/// Nothing configured: every line says `default`, and the plain output is
/// the origin output without its suffixes.
#[tokio::test]
async fn config_show_origin_defaults() {
    let root = repo("cli-config-show-origin-defaults");
    let plain = run_config(&root, &["pin-toolkit", "config", "show"], &[]).await;
    let out = run_config(&root, &["pin-toolkit", "config", "show", "--origin"], &[]).await;
    assert_eq!((out.code, out.stderr.as_str()), (0, ""));
    assert!(
        out.stdout.lines().count() >= 9 && out.stdout.lines().all(|l| l.ends_with("  # default")),
        "{}",
        out.stdout
    );
    let stripped: String = out
        .stdout
        .lines()
        .map(|l| format!("{}\n", l.strip_suffix("  # default").unwrap()))
        .collect();
    assert_eq!(stripped, plain.stdout, "the suffix is the only difference");
}

/// Every layer end to end: json and `[hosts]` from the global file, update
/// and `ignore` from the repo file, concurrency and the primary host
/// (`GITLAB_URI`) from the env, `--update-major` and `--kind` from the CLI.
/// (`ignore` cannot come from the global file; `global+repo` is pinned by
/// `show::tests::origin_plugin_lists_and_kinds`.)
#[tokio::test]
async fn config_show_origin_global_repo_env_cli() {
    let root = repo("cli-config-show-origin-layers");
    write(
        &root,
        "xdg/pin-toolkit/config.toml",
        "version = 1\njson = true\n\
         [hosts.\"gitlab.example.com\"]\nkind = \"gitlab\"\ntoken-env = \"GITLAB_EXAMPLE_TOKEN\"\n",
    );
    write(
        &root,
        ".pin-toolkit.toml",
        "version = 1\n[update]\nenabled = true\n[release_age]\nmin_days = 7\n\
         [plugins.gitlab-ci]\nignore = [\"registry.example.com/legacy/*\"]\n",
    );
    let xdg = root.join("xdg").display().to_string();
    let vars = [
        ("XDG_CONFIG_HOME", xdg.as_str()),
        ("PIN_TOOLKIT_CONCURRENCY", "3"),
        ("GITLAB_URI", "https://uri.example.com/"),
    ];
    let argv = [
        "pin-toolkit",
        "--update-major",
        "--kind",
        "gitlab-ci:image=off",
        "config",
        "show",
        "--origin",
    ];
    let out = run_config(&root, &argv, &vars).await;
    assert_eq!((out.code, out.stderr.as_str()), (0, ""));
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(
        lines[..9],
        [
            "config.global-file = xdg/pin-toolkit/config.toml  # global",
            "config.repo-file = .pin-toolkit.toml  # repo",
            "update.enabled = true  # repo",
            "update.major = true  # cli",
            "release-age.min-days = 7  # repo",
            "release-age.unknown = allow  # default",
            "concurrency = 3  # env",
            "json = true  # global",
            "gitlab-host = uri.example.com  # env",
        ],
        "{}",
        out.stdout
    );
    let plugin_lines: Vec<&str> = lines[9..]
        .iter()
        .copied()
        .filter(|l| l.starts_with("plugins."))
        .collect();
    for want in [
        r#"plugins.gitlab-ci.ignore = ["registry.example.com/legacy/*"]  # repo"#,
        "plugins.gitlab-ci.kinds.image = false  # cli",
    ] {
        assert!(plugin_lines.contains(&want), "{want}\n{}", out.stdout);
    }
    #[cfg(feature = "gitlab-ci")]
    for want in [
        "plugins.gitlab-ci.run = true  # default",
        "plugins.gitlab-ci.kinds.service = true  # default",
        "plugins.gitlab-ci.gitlab-hosts = []  # default",
    ] {
        assert!(plugin_lines.contains(&want), "{want}\n{}", out.stdout);
    }
    assert_eq!(
        lines[lines.len() - 2..],
        [
            "hosts.gitlab.example.com.kind = gitlab  # global",
            "hosts.gitlab.example.com.token-env = GITLAB_EXAMPLE_TOKEN  # global",
        ]
    );
}

/// The CLI beats env, repo and global.
#[tokio::test]
async fn config_show_origin_cli_over_env_repo_global() {
    let root = repo("cli-config-show-origin-cli");
    write(
        &root,
        "xdg/pin-toolkit/config.toml",
        "version = 1\njson = false\nconcurrency = 2\n",
    );
    write(
        &root,
        ".pin-toolkit.toml",
        "version = 1\n[update]\nenabled = false\n[release_age]\nmin_days = 7\n",
    );
    let xdg = root.join("xdg").display().to_string();
    let vars = [
        ("XDG_CONFIG_HOME", xdg.as_str()),
        ("PIN_TOOLKIT_JSON", "0"),
        ("PIN_TOOLKIT_MIN_AGE_DAYS", "9"),
        ("PIN_TOOLKIT_CONCURRENCY", "3"),
    ];
    let argv = [
        "pin-toolkit",
        "config",
        "show",
        "--origin",
        "--update",
        "--json",
        "--concurrency",
        "4",
        "--min-age-days",
        "30",
    ];
    let out = run_config(&root, &argv, &vars).await;
    assert_eq!((out.code, out.stderr.as_str()), (0, ""));
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(
        lines[2..8],
        [
            "update.enabled = true  # cli",
            "update.major = false  # default",
            "release-age.min-days = 30  # cli",
            "release-age.unknown = allow  # default",
            "concurrency = 4  # cli",
            "json = true  # cli",
        ],
        "{}",
        out.stdout
    );
    // Without the CLI the env wins over the files.
    let out = run_config(&root, &["pin-toolkit", "config", "show", "--origin"], &vars).await;
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(
        (lines[2], lines[4], lines[6], lines[7]),
        (
            "update.enabled = false  # repo",
            "release-age.min-days = 9  # env",
            "concurrency = 3  # env",
            "json = false  # env"
        )
    );
}

/// `gitlab-host` says where the primary comes from: `env` for
/// `GITLAB_URI`/`CI_SERVER_HOST`/`PIN_TOOLKIT_GITLAB_HOST`, `cli` for
/// `--gitlab-host`, else `default`.
#[tokio::test]
async fn config_show_origin_gitlab_host() {
    let root = repo("cli-config-show-origin-host");
    let line = |argv: &'static [&'static str], vars: &'static [(&'static str, &'static str)]| {
        let root = root.clone();
        async move {
            let argv: Vec<&str> = ["pin-toolkit", "config", "show", "--origin"]
                .iter()
                .chain(argv)
                .copied()
                .collect();
            let out = run_config(&root, &argv, vars).await;
            assert_eq!(out.code, 0, "{argv:?} stderr: {}", out.stderr);
            out.stdout
                .lines()
                .find(|l| l.starts_with("gitlab-host = "))
                .map(String::from)
                .unwrap()
        }
    };
    assert_eq!(line(&[], &[]).await, "gitlab-host = gitlab.com  # default");
    assert_eq!(
        line(&[], &[("GITLAB_URI", "https://uri.example.com:8443/x")]).await,
        "gitlab-host = uri.example.com:8443  # env"
    );
    assert_eq!(
        line(&[], &[("CI_SERVER_HOST", "ci.example.com")]).await,
        "gitlab-host = ci.example.com  # env"
    );
    assert_eq!(
        line(&[], &[("GITLAB_URI", "https://gitlab.com/")]).await,
        "gitlab-host = gitlab.com  # env",
        "an env value, even if it names the default"
    );
    assert_eq!(
        line(&[], &[("PIN_TOOLKIT_GITLAB_HOST", "pt.example.com")]).await,
        "gitlab-host = pt.example.com  # env"
    );
    assert_eq!(
        line(
            &["--gitlab-host", "cli.example.com"],
            &[("GITLAB_URI", "https://uri.example.com/")]
        )
        .await,
        "gitlab-host = cli.example.com  # cli"
    );
}

/// `--origin` is a `config show` flag and the rest of the rules hold: bad
/// config exits 2 with nothing on stdout.
#[tokio::test]
async fn config_show_origin_bad_config_exits_2() {
    let root = repo("cli-config-show-origin-bad");
    write(&root, ".pin-toolkit.toml", "version = 2\n");
    let out = run_config(&root, &["pin-toolkit", "config", "show", "--origin"], &[]).await;
    assert_eq!((out.code, out.stdout.as_str()), (2, ""));
    assert!(
        out.stderr.starts_with("pin-toolkit: .pin-toolkit.toml: "),
        "stderr: {}",
        out.stderr
    );
    let out = run_config(&root, &["pin-toolkit", "--origin", "config", "show"], &[]).await;
    assert_eq!((out.code, out.stdout.as_str()), (2, ""));
    assert!(out.stderr.starts_with("error:"), "stderr: {}", out.stderr);
}
