/// 40-hex lowercase commit SHA (M3 constraint 7): already pinned, accepted
/// as-is without an API call. Shared by the GitLab client and the plugin
/// policy (and, later, github-actions).
pub fn is_commit_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_commit_sha_table() {
        let sha = "e3262fdd0914fa823210cdb79a8c421e2cef79d8";
        assert!(is_commit_sha(sha));
        assert!(!is_commit_sha(&sha.to_uppercase()));
        assert!(!is_commit_sha(&sha[..39]));
        assert!(!is_commit_sha(&format!("{sha}0")));
        assert!(!is_commit_sha(&format!("g{}", &sha[1..])));
        assert!(!is_commit_sha(""));
    }
}
