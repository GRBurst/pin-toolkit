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
    assert_eq!(out.stderr, "");

    let out = run_in(&root, &["pin-toolkit", "--version"], &[]).await;
    assert_eq!((out.code, out.stdout.as_str()), (0, "pin-toolkit 0.1.0\n"));
}
