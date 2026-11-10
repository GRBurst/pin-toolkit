//! Minimal docker versioning: a faithful port of the subset of Renovate's
//! `docker` versioning (lib/modules/versioning/docker/index.ts + generic base)
//! that `--update` needs. No I/O, no dependencies.
//!
//! Deliberate divergence from Renovate: prerelease/suffix ties compare
//! bytewise (`str::cmp`) instead of `localeCompare` — deterministic without
//! an ICU dependency.

use std::cmp::Ordering;

/// A parsed docker version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Numeric release parts (`"24.04"` → `[24, 4]`; leading zeros allowed).
    pub release: Vec<u64>,
    /// Text after the numeric core, before the first `-` (`"1.2.3rc1"` →
    /// `"rc1"`); `""` when absent.
    pub prerelease: String,
    /// Everything after the first `-` (`"3.20-alpine"` → `"alpine"`);
    /// `""` when absent. A suffix does NOT make a version unstable.
    pub suffix: String,
}

impl Version {
    /// Stable = no prerelease (Renovate's generic `isStable`).
    pub fn is_stable(&self) -> bool {
        self.prerelease.is_empty()
    }

    /// Renovate docker `isCompatible`: same suffix AND same release length.
    pub fn is_compatible(&self, other: &Version) -> bool {
        self.suffix == other.suffix && self.release.len() == other.release.len()
    }

    /// The major component.
    pub fn major(&self) -> Option<u64> {
        self.release.first().copied()
    }
}

/// Parse a docker tag into a [`Version`], or `None` when it is not a version.
///
/// - Rejects empty input and commit hashes (`[a-f0-9]{7,40}` with at least
///   one letter; a pure-digit string like `"1234567"` is a valid version).
/// - Strips one leading `v`.
/// - Splits at the FIRST `-` into `(prefix, suffix)`.
/// - The numeric core of `prefix` is the longest leading `\d+(\.\d+)*` run
///   (leading zeros allowed); the rest of `prefix` is the prerelease and
///   must be entirely `[A-Za-z0-9_]`.
pub fn parse(tag: &str) -> Option<Version> {
    if tag.is_empty() || is_commit_hash(tag) {
        return None;
    }
    let t = tag.strip_prefix('v').unwrap_or(tag);
    let (prefix, suffix) = match t.split_once('-') {
        Some((p, s)) => (p, s.to_string()),
        None => (t, String::new()),
    };
    let core_end = numeric_core_len(prefix)?;
    let prerelease = &prefix[core_end..];
    if !prerelease
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return None;
    }
    let release = prefix[..core_end]
        .split('.')
        .map(|p| p.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    Some(Version {
        release,
        prerelease: prerelease.to_string(),
        suffix,
    })
}

/// Renovate's commit-hash exclusion: `[a-f0-9]{7,40}` with at least one
/// letter (a pure-digit string like `"1234567"` stays a valid version).
fn is_commit_hash(s: &str) -> bool {
    (7..=40).contains(&s.len())
        && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && s.bytes().any(|b| b.is_ascii_lowercase())
}

/// Length of the leading `\d+(\.\d+)*` run in `s`, anchored at position 0.
/// `None` when `s` does not start with a digit or a dot is left dangling
/// (a dot not followed by a digit can be neither version nor prerelease
/// text — mirrors the anchored regex of the original).
fn numeric_core_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.is_empty() || !b[0].is_ascii_digit() {
        return None;
    }
    let mut i = 0;
    loop {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
            i += 1;
        } else {
            if i < b.len() && b[i] == b'.' {
                return None;
            }
            return Some(i);
        }
    }
}

/// Compare two docker tags (docker flavor of Renovate's `_compare`):
/// release part-by-part, where a missing part means the SHORTER version is
/// BIGGER (`2.1 > 2.1.1`); then prerelease (absent > present, then
/// bytewise); then suffix in REVERSED order (the lexicographically SMALLER
/// suffix is the bigger version — Renovate returns
/// `suffix2.localeCompare(suffix1)`). `None` when either side is unparseable.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (va, vb) = (parse(a)?, parse(b)?);
    let len = va.release.len().max(vb.release.len());
    for i in 0..len {
        match (va.release.get(i), vb.release.get(i)) {
            (None, _) => return Some(Ordering::Greater), // shorter is bigger
            (_, None) => return Some(Ordering::Less),
            (Some(x), Some(y)) if x != y => return Some(x.cmp(y)),
            _ => {}
        }
    }
    if va.prerelease != vb.prerelease {
        return match (va.prerelease.is_empty(), vb.prerelease.is_empty()) {
            (true, false) => Some(Ordering::Greater),
            (false, true) => Some(Ordering::Less),
            _ => Some(va.prerelease.cmp(&vb.prerelease)),
        };
    }
    if va.suffix != vb.suffix {
        // Renovate returns `suffix2.localeCompare(suffix1)`: the
        // lexicographically SMALLER suffix is the BIGGER version.
        return Some(vb.suffix.cmp(&va.suffix));
    }
    Some(Ordering::Equal)
}

/// The highest stable tag compatible with `current` (same suffix, same
/// release length, same major unless `allow_major`), or `None` when no
/// candidate is strictly greater than `current` — or `current` itself is
/// not a version (never bumps).
pub fn pick_latest_stable(current: &str, tags: &[String], allow_major: bool) -> Option<String> {
    let cur = parse(current)?;
    let mut best: Option<&String> = None;
    for t in tags {
        let v = match parse(t) {
            Some(v) if v.is_stable() && v.is_compatible(&cur) => v,
            _ => continue,
        };
        if !allow_major && v.major() != cur.major() {
            continue;
        }
        let beats = match best {
            None => compare(t, current).unwrap_or(Ordering::Equal) == Ordering::Greater,
            Some(b) => compare(t, b).unwrap_or(Ordering::Equal) == Ordering::Greater,
        };
        if beats {
            best = Some(t);
        }
    }
    best.cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_table() {
        type Case = (&'static str, Option<(Vec<u64>, &'static str, &'static str)>);
        let cases: &[Case] = &[
            ("3.20", Some((vec![3, 20], "", ""))),
            ("24", Some((vec![24], "", ""))),
            ("24.04", Some((vec![24, 4], "", ""))),
            ("1.2.3", Some((vec![1, 2, 3], "", ""))),
            ("v1.2.3", Some((vec![1, 2, 3], "", ""))),
            ("1.2.3-rc1", Some((vec![1, 2, 3], "", "rc1"))), // dash → suffix (stable), not prerelease
            ("3.20-alpine", Some((vec![3, 20], "", "alpine"))),
            ("1.2.3rc1", Some((vec![1, 2, 3], "rc1", ""))),
            ("1.2.3-rc-1", Some((vec![1, 2, 3], "", "rc-1"))),
            ("1.2.x", None), // dot before the prerelease is unmatchable by the anchored regex
            ("1234567", Some((vec![1234567], "", ""))),
            ("", None),
            ("latest", None),
            ("main", None),
            ("stable", None),
            ("1..2", None),
            ("abc", None),
            ("abcdef1", None),
            (".1", None),
            ("1.", None),
        ];
        for (tag, expected) in cases {
            let parsed = parse(tag);
            let got = parsed
                .as_ref()
                .map(|v| (v.release.clone(), v.prerelease.as_str(), v.suffix.as_str()));
            assert_eq!(got, *expected, "parse({tag:?})");
        }
    }

    #[test]
    fn stability() {
        assert!(parse("3.20").unwrap().is_stable());
        assert!(parse("3.20-alpine").unwrap().is_stable());
        assert!(parse("3.20.5-rc0").unwrap().is_stable()); // dash-suffix is stable in Renovate docker versioning
        assert!(!parse("1.2.3rc1").unwrap().is_stable()); // glued prerelease is not
    }

    #[test]
    fn compatibility() {
        let a = parse("3.20").unwrap();
        assert!(a.is_compatible(&parse("3.24").unwrap()));
        assert!(!a.is_compatible(&parse("3.24.0").unwrap()));
        assert!(!a.is_compatible(&parse("3.20-alpine").unwrap()));
        assert!(
            parse("3.20-alpine")
                .unwrap()
                .is_compatible(&parse("3.24-alpine").unwrap())
        );
    }

    #[test]
    fn compare_table() {
        use Ordering::*;
        type Case = (&'static str, &'static str, Ordering);
        let cases: &[Case] = &[
            ("2.1", "2.1.1", Greater),
            ("2.1.1", "2.1", Less),
            ("2.1", "2.2", Less),
            ("3.20.5", "3.20", Less),
            ("1.2.3-rc1", "1.2.3", Less), // suffixed < plain (smaller suffix wins)
            ("1.2.3", "1.2.3-rc1", Greater),
            ("1.2.3-rc1", "1.2.3-rc2", Greater), // suffix order (rc1 < rc2 lexically)
            ("1.2.3rc1", "1.2.3rc2", Less),      // genuine prereleases compare directly
            ("3.20-ubuntu", "3.20-alpine", Less), // smaller suffix wins (Renovate: suffix2.localeCompare(suffix1))
            ("3.20", "3.20", Equal),
        ];
        for (a, b, want) in cases {
            assert_eq!(compare(a, b), Some(*want), "{a} vs {b}");
        }
        assert_eq!(compare("latest", "3.20"), None);
    }

    #[test]
    fn pick_latest_stable_table() {
        let t = |s: &str| s.to_string();
        assert_eq!(
            pick_latest_stable(
                "3.20",
                &[
                    t("3.16"),
                    t("3.20"),
                    t("3.24"),
                    t("3.24.0"),
                    t("4.0"),
                    t("3.25-rc1"),
                    t("3.20-alpine")
                ],
                false
            ),
            Some("3.24".to_string())
        );
        assert_eq!(
            pick_latest_stable("3.20", &[t("3.24"), t("4.2"), t("4.0")], true),
            Some("4.2".to_string())
        );
        assert_eq!(
            pick_latest_stable("24", &[t("24"), t("24.0.1"), t("29")], false),
            None
        );
        assert_eq!(
            pick_latest_stable("24", &[t("24"), t("24.0.1"), t("29")], true),
            Some("29".to_string())
        );
        assert_eq!(
            pick_latest_stable("3.20", &[t("3.19"), t("3.20")], true),
            None
        );
        assert_eq!(pick_latest_stable("latest", &[t("3.24")], true), None);
        assert_eq!(pick_latest_stable("3.20", &[], true), None);
    }
}
