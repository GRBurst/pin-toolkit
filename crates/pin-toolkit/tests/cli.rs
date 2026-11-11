//! The CLI front-end through `execute` (spec §6.6).

mod support;

use support::{repo, run_in};

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
