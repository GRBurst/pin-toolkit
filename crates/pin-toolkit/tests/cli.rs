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
