//! Inputs of a run (spec §6.3) and their routing to plugins (§6.2; T-D10,
//! T-D23): explicit files/directories or the git root, `.gitignore`
//! respected, sorted by display path.

use std::path::{Component, Path, PathBuf};

use globset::GlobSet;
use ignore::WalkBuilder;
use pin_core::report::ErrorEntry;

use crate::config::Fatal;

/// The first ancestor of `cwd` (incl.) that contains a `.git` entry.
pub fn git_root(cwd: &Path) -> Option<PathBuf> {
    git_root_with(cwd, |p| p.exists())
}

/// [`git_root`] with the existence check injected (tests).
fn git_root_with(cwd: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|a| exists(&a.join(".git")))
        .map(Path::to_path_buf)
}

/// Lexical normalisation: drops `.`, pops on `..` (never above `/`); no
/// file-system access (PR-M9).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            c => out.push(c),
        }
    }
    out
}

/// `p`'s components, `/`-joined.
fn slashed(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// `to` relative to `from` (both normalised, absolute), `/`-joined.
fn relative(from: &Path, to: &Path) -> String {
    let (f, t): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let up = f[common..].iter().map(|_| "..".to_string());
    let down = t[common..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    up.chain(down).collect::<Vec<_>>().join("/")
}

/// `path` as shown to the user: relative to `cwd` (both normalised), `.`
/// for the cwd itself. Walked files, walk errors and the root config file
/// share this rule (review-2 N1).
pub fn shown(cwd: &Path, path: &Path) -> String {
    match relative(&normalize(cwd), &normalize(path)) {
        d if d.is_empty() => ".".to_string(),
        d => d,
    }
}

/// One input file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// Normalised `cwd.join(p)`: the file to read and write.
    pub path: PathBuf,
    /// As passed (explicit), else relative to the cwd.
    pub display: String,
    /// Relative to its base (see [`inputs`]), `/`-joined: what the
    /// plugins' globs match.
    pub rel: Option<String>,
    /// Named on the command line (not found by a walk).
    pub explicit: bool,
}

/// Something a walk could not read (review-2 L3).
struct Issue {
    path: Option<PathBuf>,
    line: Option<u64>,
    message: String,
    /// An invalid ignore-file line (a warning), else an I/O-type error.
    ignore_file: bool,
}

/// Flatten `err` into issues; the innermost path and line apply.
fn issues(err: &ignore::Error, path: Option<&Path>, line: Option<u64>, out: &mut Vec<Issue>) {
    use ignore::Error as E;
    let mut push = |message: String, ignore_file: bool| {
        out.push(Issue {
            path: path.map(Path::to_path_buf),
            line,
            message,
            ignore_file,
        })
    };
    match err {
        E::Partial(errs) => errs.iter().for_each(|e| issues(e, path, line, out)),
        E::WithLineNumber { line, err } => issues(err, path, Some(*line), out),
        E::WithPath { path, err } => issues(err, Some(path), line, out),
        E::WithDepth { err, .. } => issues(err, path, line, out),
        // walkdir wraps the OS error (its text repeats the absolute path).
        E::Io(e) => push(
            e.get_ref()
                .and_then(|inner| inner.source())
                .map_or_else(|| e.to_string(), ToString::to_string),
            false,
        ),
        E::Glob { .. } => push(err.to_string(), true),
        other => push(other.to_string(), line.is_some()),
    }
}

/// The files under `dir` (`.gitignore`, `.ignore` and global excludes
/// respected; hidden files kept; `.git` skipped), normalised, and what
/// could not be read.
fn walk(dir: &Path) -> (Vec<PathBuf>, Vec<Issue>) {
    let (mut files, mut found) = (Vec::new(), Vec::new());
    let entries = WalkBuilder::new(dir)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for entry in entries {
        match entry {
            Ok(e) => {
                // A directory's ignore files that only partly parsed.
                if let Some(err) = e.error() {
                    issues(err, None, None, &mut found);
                }
                if e.file_type().is_some_and(|t| t.is_file()) {
                    files.push(normalize(e.path()));
                }
            }
            Err(err) => issues(&err, None, None, &mut found),
        }
    }
    (files, found)
}

/// The inputs of a run and what the walks could not read (review-2 L3):
/// I/O-type errors (an unreadable directory or entry) are per-file errors
/// (exit 2); an invalid ignore-file line is a warning (`{file}:{line}:
/// {message}`; git tolerates it too). Paths are shown relative to the cwd.
#[derive(Debug, Default, PartialEq)]
pub struct Inputs {
    pub files: Vec<Input>,
    pub errors: Vec<ErrorEntry>,
    pub warnings: Vec<String>,
}

/// The inputs of a run: `paths` (relative to `cwd`; a directory is walked)
/// or, without paths, the walked `root`. `rel` is relative to a base
/// directory: `root` for whatever lies under it; outside it, the git root
/// of the walked directory, else the directory itself (review-2 M1), and
/// for an explicit file the same rule applied to its parent directory (M5d
/// final L1). Unique by normalised path: the first spelling in argument
/// order gives the display, a file named explicitly stays explicit
/// (review-2 M2), and the outermost base gives `rel` (the bases of one
/// file are all its ancestors), so the claim does not depend on argument
/// order. Sorted by display.
pub fn inputs(cwd: &Path, root: &Path, paths: &[PathBuf]) -> Inputs {
    inputs_with(cwd, root, paths, |p| p.exists())
}

/// [`inputs`] with the `.git` existence check injected (tests).
fn inputs_with(
    cwd: &Path,
    root: &Path,
    paths: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Inputs {
    let (cwd, root) = (normalize(cwd), normalize(root));
    let rel_to = |base: &Path, path: &Path| path.strip_prefix(base).ok().map(slashed);
    let base_of = |dir: &Path| {
        if dir.starts_with(&root) {
            root.clone()
        } else {
            git_root_with(dir, &exists).unwrap_or_else(|| dir.to_path_buf())
        }
    };
    let mut found = Inputs::default();
    let mut walked = |dir: &Path| {
        let base = base_of(dir);
        let (files, issues) = walk(dir);
        for i in issues {
            let file = shown(&cwd, i.path.as_deref().unwrap_or(dir));
            if i.ignore_file {
                found.warnings.push(match i.line {
                    Some(line) => format!("{file}:{line}: {}", i.message),
                    None => format!("{file}: {}", i.message),
                });
            } else {
                found.errors.push(ErrorEntry {
                    file,
                    line: i.line.and_then(|l| usize::try_from(l).ok()),
                    message: i.message,
                });
            }
        }
        files
            .into_iter()
            .map(|path| Input {
                display: relative(&cwd, &path),
                rel: rel_to(&base, &path),
                path,
                explicit: false,
            })
            .collect::<Vec<_>>()
    };
    let mut all: Vec<Input> = Vec::new();
    if paths.is_empty() {
        all.extend(walked(&root));
    }
    for p in paths {
        let path = normalize(&cwd.join(p));
        if path.is_dir() {
            all.extend(walked(&path));
        } else {
            let base = path.parent().map_or_else(|| root.clone(), base_of);
            all.push(Input {
                display: p.to_string_lossy().into_owned(),
                rel: rel_to(&base, &path),
                path,
                explicit: true,
            });
        }
    }
    let mut unique: Vec<Input> = Vec::with_capacity(all.len());
    let mut index = std::collections::HashMap::new();
    for input in all {
        match index.get(&input.path) {
            Some(&i) => {
                let first: &mut Input = &mut unique[i];
                first.explicit |= input.explicit;
                // The longer `rel` is relative to the outer base (L1).
                let depth = |r: &Option<String>| r.as_ref().map_or(0, |r| r.split('/').count());
                if depth(&input.rel) > depth(&first.rel) {
                    first.rel = input.rel;
                }
            }
            None => {
                index.insert(input.path.clone(), unique.len());
                unique.push(input);
            }
        }
    }
    unique.sort_by(|a, b| a.display.cmp(&b.display));
    found.files = unique;
    found
}

/// Inputs routed to plugins, and walked files claimed by 2+ plugins
/// (per-file errors under the first claimant; PR-L11).
#[derive(Debug)]
pub struct Routed {
    pub routes: Vec<(Input, &'static str)>,
    pub conflicts: Vec<(&'static str, ErrorEntry)>,
}

/// `{file} is claimed by {p1} and {p2} (…)`.
fn claimed_by(file: &str, ids: &[&str]) -> String {
    let (last, rest) = ids.split_last().expect("2+ claimants");
    format!(
        "{file} is claimed by {} and {last} (narrow their `files` globs, or choose one: pin-toolkit <plugin> {file})",
        rest.join(", ")
    )
}

/// Each input to the one plugin whose globs match its `rel`. Explicit:
/// `forced` (a subcommand/alias) claims it; else 0 claims or 2+ claims are
/// fatal. Walked: 0 claims ⇒ dropped; 2+ ⇒ a conflict
/// under the first claimant in `claims` order (T-D10).
pub fn route(
    inputs: Vec<Input>,
    claims: &[(&'static str, &GlobSet)],
    forced: Option<&'static str>,
) -> Result<Routed, Fatal> {
    let mut routed = Routed {
        routes: Vec::new(),
        conflicts: Vec::new(),
    };
    for input in inputs {
        if let (true, Some(id)) = (input.explicit, forced) {
            routed.routes.push((input, id));
            continue;
        }
        let ids: Vec<&'static str> = match &input.rel {
            Some(rel) => claims
                .iter()
                .filter(|(_, globs)| globs.is_match(rel))
                .map(|(id, _)| *id)
                .collect(),
            None => Vec::new(),
        };
        match (ids.as_slice(), input.explicit) {
            ([id], _) => routed.routes.push((input, *id)),
            ([], false) => {}
            ([], true) => {
                let file = &input.display;
                return Err(Fatal(format!(
                    "no plugin claims {file} (choose one: pin-toolkit <plugin> {file})"
                )));
            }
            (_, true) => return Err(Fatal(claimed_by(&input.display, &ids))),
            ([first, ..], false) => {
                let message = claimed_by(&input.display, &ids);
                routed.conflicts.push((
                    *first,
                    ErrorEntry {
                        file: input.display,
                        line: None,
                        message,
                    },
                ));
            }
        }
    }
    Ok(routed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use globset::{GlobBuilder, GlobSetBuilder};

    /// A fresh `$CARGO_MANIFEST_DIR/.tmp/<name>-<pid>/` (constraint 8).
    fn tmp(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A repo (with `.git/`) holding `files` (empty contents).
    fn repo(dir: &Path, files: &[&str]) {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        for f in files {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
    }

    fn set(globs: &[&str]) -> GlobSet {
        let mut b = GlobSetBuilder::new();
        for g in globs {
            b.add(GlobBuilder::new(g).literal_separator(true).build().unwrap());
        }
        b.build().unwrap()
    }

    fn gitlab_ci() -> GlobSet {
        set(&pin_gitlab_ci::DEFAULT_FILES)
    }

    fn paths(ps: &[&str]) -> Vec<PathBuf> {
        ps.iter().map(PathBuf::from).collect()
    }

    /// `(display, plugin)` per route.
    fn shown(r: &Routed) -> Vec<(String, &'static str)> {
        r.routes
            .iter()
            .map(|(i, p)| (i.display.clone(), *p))
            .collect()
    }

    const TREE: [&str; 7] = [
        ".git/x.gitlab-ci.yml",
        ".gitignore",
        ".gitlab-ci.yml",
        "ci/.gitlab-ci.yml",
        ".gitlab/ci/a.yml",
        "ignored/x.gitlab-ci.yml",
        "docs/readme.yml",
    ];

    fn tree(name: &str) -> PathBuf {
        let root = tmp(name);
        repo(&root, &TREE);
        std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        root
    }

    fn claimed_by(file: &str, a: &str, b: &str) -> String {
        format!(
            "{file} is claimed by {a} and {b} (narrow their `files` globs, or choose one: pin-toolkit <plugin> {file})"
        )
    }

    fn no_claim(file: &str) -> Fatal {
        Fatal(format!(
            "no plugin claims {file} (choose one: pin-toolkit <plugin> {file})"
        ))
    }

    #[test]
    fn root_walk_respects_gitignore_and_hidden() {
        let root = tree("root_walk_respects_gitignore_and_hidden");
        let gl = gitlab_ci();
        let r = route(inputs(&root, &root, &[]).files, &[("gitlab-ci", &gl)], None).unwrap();
        assert_eq!(
            shown(&r),
            [
                (".gitlab-ci.yml".to_string(), "gitlab-ci"),
                (".gitlab/ci/a.yml".to_string(), "gitlab-ci"),
                ("ci/.gitlab-ci.yml".to_string(), "gitlab-ci"),
            ]
        );
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn display_relative_to_cwd() {
        let root = tree("display_relative_to_cwd");
        let cwd = root.join("ci");
        let gl = gitlab_ci();
        let r = route(inputs(&cwd, &root, &[]).files, &[("gitlab-ci", &gl)], None).unwrap();
        let displays: Vec<_> = shown(&r).into_iter().map(|(d, _)| d).collect();
        assert_eq!(
            displays,
            ["../.gitlab-ci.yml", "../.gitlab/ci/a.yml", ".gitlab-ci.yml"]
        );
        let rels: Vec<_> = r.routes.iter().map(|(i, _)| i.rel.clone()).collect();
        assert_eq!(
            rels,
            [
                Some(".gitlab-ci.yml".to_string()),
                Some(".gitlab/ci/a.yml".to_string()),
                Some("ci/.gitlab-ci.yml".to_string()),
            ]
        );
        for (i, _) in &r.routes {
            assert!(i.path.is_file(), "{}", i.path.display());
            assert!(!i.explicit);
        }
    }

    #[test]
    fn explicit_file_claim_rules() {
        let root = tree("explicit_file_claim_rules");
        let gl = gitlab_ci();
        let readme = paths(&["docs/readme.yml"]);
        let all = route(
            inputs(&root, &root, &readme).files,
            &[("gitlab-ci", &gl)],
            None,
        );
        assert_eq!(all.unwrap_err(), no_claim("docs/readme.yml"));

        let (p1, p2) = (set(&["**/*.yml"]), set(&["docs/*.yml"]));
        let two = route(
            inputs(&root, &root, &readme).files,
            &[("p1", &p1), ("p2", &p2)],
            None,
        );
        assert_eq!(
            two.unwrap_err(),
            Fatal(claimed_by("docs/readme.yml", "p1", "p2"))
        );

        let forced = route(
            inputs(&root, &root, &readme).files,
            &[("gitlab-ci", &gl)],
            Some("gitlab-ci"),
        )
        .unwrap();
        assert_eq!(
            shown(&forced),
            [("docs/readme.yml".to_string(), "gitlab-ci")]
        );
        assert!(forced.routes[0].0.explicit);

        let twice = paths(&[".gitlab-ci.yml", ".gitlab-ci.yml"]);
        let once = route(
            inputs(&root, &root, &twice).files,
            &[("gitlab-ci", &gl)],
            None,
        )
        .unwrap();
        assert_eq!(shown(&once), [(".gitlab-ci.yml".to_string(), "gitlab-ci")]);
    }

    #[test]
    fn explicit_dir_is_walked() {
        let root = tree("explicit_dir_is_walked");
        let gl = gitlab_ci();
        let r = route(
            inputs(&root, &root, &paths(&["ci/"])).files,
            &[("gitlab-ci", &gl)],
            None,
        )
        .unwrap();
        assert_eq!(shown(&r), [("ci/.gitlab-ci.yml".to_string(), "gitlab-ci")]);
        assert_eq!(r.routes[0].0.rel.as_deref(), Some("ci/.gitlab-ci.yml"));
        assert!(!r.routes[0].0.explicit);
        // `.gitlab/ci/**/*.yml` matches only the root-relative `.gitlab/ci/a.yml`
        // (relative to the walked directory it would be `ci/a.yml`).
        let r = route(
            inputs(&root, &root, &paths(&[".gitlab"])).files,
            &[("gitlab-ci", &gl)],
            None,
        )
        .unwrap();
        assert_eq!(shown(&r), [(".gitlab/ci/a.yml".to_string(), "gitlab-ci")]);
    }

    #[test]
    fn no_git_root_falls_back_to_cwd() {
        assert_eq!(git_root_with(Path::new("/a/b"), |_| false), None);
        assert_eq!(
            git_root_with(Path::new("/a/b"), |p| p == Path::new("/a/.git")),
            Some(PathBuf::from("/a"))
        );
        let root = tree("no_git_root_falls_back_to_cwd");
        let cwd = root.join("ci");
        let walked = inputs(&cwd, &cwd, &[]).files;
        let shown: Vec<_> = walked
            .iter()
            .map(|i| (i.display.as_str(), i.rel.as_deref()))
            .collect();
        assert_eq!(shown, [(".gitlab-ci.yml", Some(".gitlab-ci.yml"))]);
    }

    #[test]
    fn explicit_parent_path_from_subdir() {
        let tmp = tmp("explicit_parent_path_from_subdir");
        let root = tmp.join("repo");
        repo(&root, &TREE);
        std::fs::write(tmp.join("outside.gitlab-ci.yml"), "").unwrap();
        let cwd = root.join("ci");
        let gl = gitlab_ci();
        let claims = [("gitlab-ci", &gl)];

        let up = inputs(&cwd, &root, &paths(&["../.gitlab-ci.yml"])).files;
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].display, "../.gitlab-ci.yml");
        assert_eq!(up[0].rel.as_deref(), Some(".gitlab-ci.yml"));
        assert_eq!(up[0].path, root.join(".gitlab-ci.yml"));
        let r = route(up, &claims, None).unwrap();
        assert_eq!(shown(&r), [("../.gitlab-ci.yml".to_string(), "gitlab-ci")]);

        // M5d final L1: an explicit file outside the root is matched like a
        // directory there: relative to its parent's git root, else to its
        // parent (the search stops at `tmp`: this repository has a `.git`).
        let exists = |p: &Path| p.starts_with(&tmp) && p.exists();
        let out = paths(&["../../outside.gitlab-ci.yml"]);
        let outside = inputs_with(&cwd, &root, &out, exists).files;
        assert_eq!(outside.len(), 1);
        assert_eq!(outside[0].rel.as_deref(), Some("outside.gitlab-ci.yml"));
        assert_eq!(outside[0].path, tmp.join("outside.gitlab-ci.yml"));
        assert_eq!(
            shown(&route(outside, &claims, None).unwrap()),
            [("../../outside.gitlab-ci.yml".to_string(), "gitlab-ci")]
        );
        std::fs::write(tmp.join("outside.yml"), "").unwrap();
        let unmatched = paths(&["../../outside.yml"]);
        assert_eq!(
            route(
                inputs_with(&cwd, &root, &unmatched, exists).files,
                &claims,
                None
            )
            .unwrap_err(),
            no_claim("../../outside.yml")
        );
        let forced = route(
            inputs_with(&cwd, &root, &unmatched, exists).files,
            &claims,
            Some("gitlab-ci"),
        )
        .unwrap();
        assert_eq!(
            shown(&forced),
            [("../../outside.yml".to_string(), "gitlab-ci")]
        );
    }

    /// Review-2 M1: an explicit directory outside the git root is walked
    /// with globs relative to its own git root (else the directory itself),
    /// displays relative to the cwd.
    #[test]
    fn explicit_dir_outside_the_root() {
        let tmp = tmp("explicit_dir_outside_the_root");
        let root = tmp.join("repo");
        repo(&root, &TREE);
        let other = tmp.join("other");
        repo(
            &other,
            &[".gitlab-ci.yml", ".gitlab/ci/a.yml", "docs/readme.yml"],
        );
        let plain = tmp.join("plain");
        for f in [
            ".gitlab-ci.yml",
            "sub/x.gitlab-ci.yml",
            "ci/a.yml",
            ".gitlab/ci/b.yml",
        ] {
            std::fs::create_dir_all(plain.join(f).parent().unwrap()).unwrap();
            std::fs::write(plain.join(f), "").unwrap();
        }
        let cwd = root.join("ci");
        let gl = gitlab_ci();
        let claims = [("gitlab-ci", &gl)];
        // The git-root search stops at `tmp` (this repository has a `.git`).
        let exists = |p: &Path| p.starts_with(&tmp) && p.exists();
        let walk = |ps: &[&str]| {
            let ins = inputs_with(&cwd, &root, &paths(ps), exists).files;
            let rels: Vec<_> = ins.iter().map(|i| i.rel.clone()).collect();
            (shown(&route(ins, &claims, None).unwrap()), rels)
        };
        let routes = |v: &[&str]| -> Vec<(String, &'static str)> {
            v.iter().map(|d| (d.to_string(), "gitlab-ci")).collect()
        };
        let some =
            |v: &[&str]| -> Vec<Option<String>> { v.iter().map(|r| Some(r.to_string())).collect() };

        let (r, rels) = walk(&["../../other"]);
        assert_eq!(
            r,
            routes(&["../../other/.gitlab-ci.yml", "../../other/.gitlab/ci/a.yml"])
        );
        assert_eq!(
            rels,
            some(&[".gitlab-ci.yml", ".gitlab/ci/a.yml", "docs/readme.yml"])
        );
        let (r, _) = walk(&["../../other/.gitlab"]);
        assert_eq!(
            r,
            routes(&["../../other/.gitlab/ci/a.yml"]),
            "relative to other's git root, not to the directory"
        );
        let (r, rels) = walk(&["../../plain"]);
        assert_eq!(
            r,
            routes(&[
                "../../plain/.gitlab-ci.yml",
                "../../plain/.gitlab/ci/b.yml",
                "../../plain/sub/x.gitlab-ci.yml"
            ])
        );
        assert_eq!(
            rels,
            some(&[
                ".gitlab-ci.yml",
                ".gitlab/ci/b.yml",
                "ci/a.yml",
                "sub/x.gitlab-ci.yml"
            ]),
            "no git root: relative to the directory itself"
        );

        // M5d final L1: an explicit file there follows the same rule (its
        // parent's git root, else its parent), and a file named both ways
        // is claimed in either argument order.
        let (r, rels) = walk(&["../../other/.gitlab/ci/a.yml"]);
        assert_eq!(r, routes(&["../../other/.gitlab/ci/a.yml"]));
        assert_eq!(rels, some(&[".gitlab/ci/a.yml"]));
        for order in [
            ["../../other", "../../other/.gitlab-ci.yml"],
            ["../../other/.gitlab-ci.yml", "../../other"],
        ] {
            let (r, _) = walk(&order);
            assert_eq!(
                r,
                routes(&["../../other/.gitlab-ci.yml", "../../other/.gitlab/ci/a.yml"]),
                "{order:?}"
            );
        }
        // Without a git root the bases differ (the directory vs the file's
        // parent): the outer one wins, whatever the order.
        let lone = inputs_with(
            &cwd,
            &root,
            &paths(&["../../plain/.gitlab/ci/b.yml"]),
            exists,
        );
        assert_eq!(lone.files[0].rel.as_deref(), Some("b.yml"));
        for order in [
            ["../../plain", "../../plain/.gitlab/ci/b.yml"],
            ["../../plain/.gitlab/ci/b.yml", "../../plain"],
        ] {
            let ins = inputs_with(&cwd, &root, &paths(&order), exists).files;
            let b: Vec<_> = ins.iter().filter(|i| i.path.ends_with("b.yml")).collect();
            assert_eq!(b.len(), 1, "{order:?}");
            assert_eq!(b[0].rel.as_deref(), Some(".gitlab/ci/b.yml"), "{order:?}");
            assert!(b[0].explicit, "{order:?}");
            assert!(
                shown(&route(ins, &claims, None).unwrap())
                    .contains(&("../../plain/.gitlab/ci/b.yml".to_string(), "gitlab-ci")),
                "{order:?}"
            );
        }
        let forced = route(
            inputs_with(&cwd, &root, &paths(&["../../other"]), exists).files,
            &claims,
            Some("gitlab-ci"),
        )
        .unwrap();
        assert_eq!(
            shown(&forced),
            routes(&["../../other/.gitlab-ci.yml", "../../other/.gitlab/ci/a.yml"]),
            "a subcommand claims walked files by globs too"
        );
    }

    #[test]
    fn same_file_two_spellings_one_route() {
        let root = tmp("same_file_two_spellings_one_route");
        repo(&root, &["a.gitlab-ci.yml", "ci/.gitlab-ci.yml"]);
        let gl = gitlab_ci();
        let spelled = paths(&[
            "a.gitlab-ci.yml",
            "./a.gitlab-ci.yml",
            "ci/../a.gitlab-ci.yml",
        ]);
        let r = route(
            inputs(&root, &root, &spelled).files,
            &[("gitlab-ci", &gl)],
            None,
        )
        .unwrap();
        assert_eq!(shown(&r), [("a.gitlab-ci.yml".to_string(), "gitlab-ci")]);
        assert_eq!(r.routes[0].0.path, root.join("a.gitlab-ci.yml"));

        // Review-2 M2: a file both walked and named keeps `explicit`, in
        // either argument order.
        std::fs::write(root.join("ci/notes.yml"), "").unwrap();
        for order in [["ci/", "ci/notes.yml"], ["ci/notes.yml", "ci/"]] {
            let ins = inputs(&root, &root, &paths(&order)).files;
            let notes: Vec<_> = ins.iter().filter(|i| i.display == "ci/notes.yml").collect();
            assert_eq!(notes.len(), 1, "{order:?}: one input");
            assert!(notes[0].explicit, "{order:?}: explicit kept");
            let forced = route(ins.clone(), &[("gitlab-ci", &gl)], Some("gitlab-ci")).unwrap();
            assert_eq!(
                shown(&forced),
                [
                    ("ci/.gitlab-ci.yml".to_string(), "gitlab-ci"),
                    ("ci/notes.yml".to_string(), "gitlab-ci"),
                ],
                "{order:?}: forced → claimed"
            );
            assert_eq!(
                route(ins, &[("gitlab-ci", &gl)], None).unwrap_err(),
                no_claim("ci/notes.yml"),
                "{order:?}: all mode → no plugin claims"
            );
        }
    }

    /// Restores a directory's mode `0o755` on drop, also when an assertion
    /// panics: a leftover `000` directory breaks the next run (M5d final N4).
    #[cfg(unix)]
    struct Restore(PathBuf);

    #[cfg(unix)]
    impl Drop for Restore {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    /// Review-2 L3: an unreadable directory is a per-file error under its
    /// display path; an invalid `.gitignore` line only a warning (its valid
    /// lines still apply); the readable files are still walked. Skipped
    /// where permissions do not apply (root).
    #[cfg(unix)]
    #[test]
    fn walk_errors_are_reported() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp("walk_errors_are_reported");
        repo(
            &root,
            &[
                ".gitlab-ci.yml",
                "ignored/x.gitlab-ci.yml",
                "ci/locked/x.gitlab-ci.yml",
            ],
        );
        std::fs::write(root.join(".gitignore"), "ignored/\n[z-a]\n").unwrap();
        let locked = root.join("ci/locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let _restore = Restore(locked.clone());
        if std::fs::read_dir(&locked).is_ok() {
            eprintln!("skipped: permissions do not apply (running as root?)");
            return;
        }
        let found = inputs(&root.join("ci"), &root, &[]);
        let displays: Vec<_> = found.files.iter().map(|i| i.display.as_str()).collect();
        assert_eq!(displays, ["../.gitignore", "../.gitlab-ci.yml"]);
        assert_eq!(
            found.errors,
            [ErrorEntry {
                file: "locked".into(),
                line: None,
                message: "Permission denied (os error 13)".into(),
            }]
        );
        assert_eq!(found.warnings.len(), 1, "{:?}", found.warnings);
        assert!(
            found.warnings[0].starts_with("../.gitignore:2: ")
                && found.warnings[0].contains("'[z-a]'"),
            "{:?}",
            found.warnings
        );
    }

    #[test]
    fn walked_file_claimed_twice_is_a_file_error() {
        let root = tmp("walked_file_claimed_twice_is_a_file_error");
        repo(&root, &["x.yml", "y.yml"]);
        let (p1, p2) = (set(&["*.yml"]), set(&["x.yml"]));
        let claims = [("p1", &p1), ("p2", &p2)];
        let r = route(inputs(&root, &root, &[]).files, &claims, None).unwrap();
        assert_eq!(shown(&r), [("y.yml".to_string(), "p1")]);
        assert_eq!(
            r.conflicts,
            [(
                "p1",
                ErrorEntry {
                    file: "x.yml".into(),
                    line: None,
                    message: claimed_by("x.yml", "p1", "p2"),
                }
            )]
        );
        let explicit = route(
            inputs(&root, &root, &paths(&["x.yml"])).files,
            &claims,
            None,
        );
        assert_eq!(
            explicit.unwrap_err(),
            Fatal(claimed_by("x.yml", "p1", "p2"))
        );
    }
}
