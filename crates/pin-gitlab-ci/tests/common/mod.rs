//! Shared harness for the fixture snapshot/property tests (M5a): hash-driven
//! fake registry + fake GitLab, the fixture list, a run helper and the
//! normalisers that make reports snapshot-stable.
#![allow(dead_code)] // each test crate uses a subset

use std::path::{Path, PathBuf};

use pin_core::edit::{Splice, apply, coalesce_comments};
use pin_core::gitlab::GitLabClient;
use pin_core::oci::{ImageRef, RegistryClient};
use pin_core::plugin::{DynPlugin, Finding, KindSet};
use pin_core::report::{Report, Row, Status};
use pin_core::run::{Ctx, ResolverEnv, UpdatePolicy, now_unix};
use pin_core::text::LineIndex;
use pin_gitlab_ci::discover::{PinTarget, discover};
use pin_gitlab_ci::{GitlabCi, KINDS, PinOptions, pin_files};
use wiremock::matchers::path_regex;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Registry hosts the fake registry answers for. Fixtures must stay within
/// this set (checked in [`pin_once`]) so no test ever touches the network.
pub const MAPPED_HOSTS: [&str; 2] = ["docker.io", "ghcr.io"];

pub struct Fakes {
    pub registry: MockServer,
    pub gitlab: MockServer,
}

pub async fn start_fakes() -> Fakes {
    let registry = MockServer::start().await;
    Mock::given(path_regex(r"^/v2/.+$"))
        .respond_with(FakeRegistry)
        .mount(&registry)
        .await;
    let gitlab = MockServer::start().await;
    Mock::given(path_regex(r"^/projects/.+$"))
        .respond_with(FakeGitLab)
        .mount(&gitlab)
        .await;
    Fakes { registry, gitlab }
}

pub fn registry_client(f: &Fakes) -> RegistryClient {
    RegistryClient::new(
        Default::default(),
        MAPPED_HOSTS
            .iter()
            .map(|h| (h.to_string(), f.registry.uri()))
            .collect(),
    )
}

pub fn gitlab_client(f: &Fakes) -> GitLabClient {
    GitLabClient::new(f.gitlab.uri(), None, false)
}

fn fnv1a64(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Deterministic lowercase hex of `len` (≤ 64) characters derived from `key`:
/// four blocks of `splitmix64(fnv1a64([k] ⧺ key))`, each as `{:016x}`.
pub fn fake_hex(key: &str, len: usize) -> String {
    assert!(len <= 64, "fake_hex: len {len} > 64");
    let mut out = String::with_capacity(64);
    for k in 0u8..4 {
        let h = fnv1a64(std::iter::once(k).chain(key.bytes()));
        out.push_str(&format!("{:016x}", splitmix64(h)));
    }
    out.truncate(len);
    out
}

/// Registry fake. Manifests: `mystery*`/`missing*` repos → 404, everything
/// else answers a digest derived from `repo:ref`. Tag lists: fixed per repo.
struct FakeRegistry;

impl Respond for FakeRegistry {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let path = req.url.path();
        let Some(rest) = path.strip_prefix("/v2/") else {
            return ResponseTemplate::new(404);
        };
        if let Some((repo, reference)) = rest.split_once("/manifests/") {
            let name = repo.rsplit('/').next().unwrap_or(repo);
            if name.starts_with("mystery") || name.starts_with("missing") {
                return ResponseTemplate::new(404);
            }
            let digest = format!("sha256:{}", fake_hex(&format!("{repo}:{reference}"), 64));
            return ResponseTemplate::new(200).insert_header("docker-content-digest", digest);
        }
        if let Some(repo) = rest.strip_suffix("/tags/list") {
            let tags: &[&str] = match repo {
                "library/alpine" => &["3.16", "3.20", "3.24"],
                "org/img" => &["v1"],
                _ => &[],
            };
            return ResponseTemplate::new(200).set_body_json(serde_json::json!({ "tags": tags }));
        }
        ResponseTemplate::new(404)
    }
}

/// GitLab fake: `/projects/{p}/repository/{tags|branches}/{ref}`. Projects
/// starting `g%2Fmissing` 404; tag-looking refs (digit first, or `v` +
/// digit) exist only as tags, everything else only as branches.
struct FakeGitLab;

fn tagish(r: &str) -> bool {
    let mut c = r.chars();
    match c.next() {
        Some(ch) if ch.is_ascii_digit() => true,
        Some('v') => c.next().is_some_and(|ch| ch.is_ascii_digit()),
        _ => false,
    }
}

impl Respond for FakeGitLab {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let path = req.url.path();
        let parts: Vec<&str> = path.split('/').collect(); // "", projects, p, repository, kind, ref
        let [_, "projects", p, "repository", kind, r] = parts[..] else {
            return ResponseTemplate::new(404);
        };
        if p.starts_with("g%2Fmissing") {
            return ResponseTemplate::new(404);
        }
        let exists = match kind {
            "tags" => tagish(r),
            "branches" => !tagish(r),
            _ => false,
        };
        if !exists {
            return ResponseTemplate::new(404);
        }
        let id = fake_hex(&format!("{p}@{r}"), 40);
        ResponseTemplate::new(200).set_body_json(serde_json::json!({ "commit": { "id": id } }))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Pin,
    Update,
}

impl Mode {
    pub fn tag(self) -> &'static str {
        match self {
            Mode::Pin => "pin",
            Mode::Update => "update",
        }
    }
}

pub struct Fixture {
    pub name: String,
    pub stem: String,
    pub text: String,
}

/// Every `tests/fixtures/*.yml`, sorted by file name.
pub fn fixtures() -> Vec<Fixture> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".yml"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let text = std::fs::read_to_string(dir.join(&name)).unwrap();
            let stem = name.trim_end_matches(".yml").to_string();
            Fixture { name, stem, text }
        })
        .collect()
}

/// `pin` for every fixture, plus `update` for `ci-update*` fixtures.
pub fn modes(name: &str) -> Vec<Mode> {
    let mut m = vec![Mode::Pin];
    if name.starts_with("ci-update") {
        m.push(Mode::Update);
    }
    m
}

/// Copy the fixture bytes to `.tmp/fixture-{tag}-{stem}-{mode}-{pid}/{name}`
/// (gitignored scratch under the crate root); returns the file path.
pub fn scratch_copy(fx: &Fixture, mode: Mode, tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        ".tmp/fixture-{tag}-{}-{}-{}",
        fx.stem,
        mode.tag(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(&fx.name);
    std::fs::write(&p, &fx.text).unwrap();
    p
}

/// Run `pin_files` over `path` against the fakes. Panics when the file holds
/// an image on a registry host the fakes do not serve (it would hit the
/// network).
pub async fn pin_once(f: &Fakes, path: &Path, mode: Mode) -> Report {
    let text = std::fs::read_to_string(path).unwrap();
    for t in discover(&text).expect("fixture parses") {
        if let PinTarget::Image { .. } = t
            && let Ok(r) = ImageRef::parse(t.value())
            && !MAPPED_HOSTS.contains(&r.host_str())
        {
            panic!(
                "fixture {}: unmapped registry host {} (would hit the network)",
                path.file_name().unwrap().to_string_lossy(),
                r.host_str()
            );
        }
    }
    let opts = PinOptions {
        concurrency: 4,
        update: matches!(mode, Mode::Update),
        ..Default::default()
    };
    pin_files(
        &[path.to_path_buf()],
        &opts,
        &registry_client(f),
        &gitlab_client(f),
    )
    .await
    .unwrap()
}

/// Every row's `file` set to `name` (scratch paths carry the PID).
pub fn normalized(report: Report, name: &str) -> Report {
    let mut rows = report.results;
    for r in &mut rows {
        r.file = name.to_string();
    }
    Report::new(rows)
}

/// `127.0.0.1:<digits>` → `127.0.0.1:[PORT]` (mock ports are random).
pub fn redact(s: &str) -> String {
    const PREFIX: &str = "127.0.0.1:";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(PREFIX) {
        let after = &rest[i + PREFIX.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        out.push_str(&rest[..i + PREFIX.len()]);
        if digits > 0 {
            out.push_str("[PORT]");
        }
        rest = &after[digits..];
    }
    out.push_str(rest);
    out
}

/// Make `\r` and a missing final newline visible; insta normalises both.
pub fn visible(text: &str) -> String {
    let mut out = text.replace('\r', "␍");
    if !text.ends_with('\n') {
        out.push_str("\n[no newline at end of file]");
    }
    out
}

/// Insertion found on one line: byte position in the O line and the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insertion {
    pub at: usize,
    pub text: String,
}

/// Compare one edited line against its original modulo the rows on it:
/// revert every row (`new` → `old`, once, in report order), then the rest
/// must equal the original or the original plus ONE inserted trailing
/// comment (`"  # …"`, no newline) — which needs a row on the line.
/// `Err` describes the mismatch.
pub fn line_diff(o_line: &str, e_line: &str, rows: &[&Row]) -> Result<Option<Insertion>, String> {
    let mut l = e_line.to_string();
    for r in rows {
        let new = r.new.as_deref().ok_or("row without `new`")?;
        if !l.contains(new) {
            return Err(format!("row `new` {new:?} not in edited line {e_line:?}"));
        }
        l = l.replacen(new, &r.old, 1);
    }
    if l == o_line {
        return Ok(None);
    }
    let mut p = l
        .bytes()
        .zip(o_line.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !(l.is_char_boundary(p) && o_line.is_char_boundary(p)) {
        p -= 1;
    }
    let describe = || format!("original {o_line:?}, edited {e_line:?}, reverted {l:?}");
    let k = l.len().checked_sub(o_line.len()).ok_or_else(describe)?;
    let s = &l[p..p + k];
    if !rows.is_empty() && s.starts_with("  # ") && !s.contains('\n') && l[p + k..] == o_line[p..] {
        Ok(Some(Insertion {
            at: p,
            text: s.to_string(),
        }))
    } else {
        Err(describe())
    }
}

/// Edits touch only reference bytes: same line count, and every line passes
/// [`line_diff`] against the rows (pinned/updated) reported for it. Returns
/// the insertion found on each line.
pub fn check_byte_exact(
    o: &str,
    e: &str,
    report: &Report,
) -> Result<Vec<Option<Insertion>>, String> {
    let ol: Vec<&str> = o.split_inclusive('\n').collect();
    let el: Vec<&str> = e.split_inclusive('\n').collect();
    if ol.len() != el.len() {
        return Err(format!("line count {} -> {}", ol.len(), el.len()));
    }
    let mut out = Vec::with_capacity(ol.len());
    for (i, (o_line, e_line)) in ol.iter().zip(&el).enumerate() {
        let rows: Vec<&Row> = report
            .results
            .iter()
            .filter(|r| {
                r.line == i + 1 && (r.status == Status::Pinned || r.status == Status::Updated)
            })
            .collect();
        out.push(line_diff(o_line, e_line, &rows).map_err(|m| format!("line {}: {m}", i + 1))?);
    }
    Ok(out)
}

/// A second run edits nothing: file unchanged, no row pinned/updated, and
/// every previously pinned/updated/ok reference is now `ok`.
pub fn check_idempotent(r1: &Report, e1: &str, r2: &Report, e2: &str) -> Result<(), String> {
    if e1 != e2 {
        return Err("second run changed the file".to_string());
    }
    if let Some(r) = r2
        .results
        .iter()
        .find(|r| r.status == Status::Pinned || r.status == Status::Updated)
    {
        return Err(format!(
            "second run {} line {} {:?}",
            r.status, r.line, r.old
        ));
    }
    let settled = |r: &Row| matches!(r.status.as_str(), "pinned" | "updated" | "ok");
    let before = r1.results.iter().filter(|r| settled(r)).count();
    let after = r2.results.iter().filter(|r| r.status == Status::Ok).count();
    if before != after {
        return Err(format!(
            "ok rows: {before} after run 1, {after} after run 2"
        ));
    }
    Ok(())
}

/// P-crlf (R2, C12): every CRLF line of `o` stays CRLF in `e`, and its
/// inserted comment (from [`check_byte_exact`]) sits before the `\r\n`.
pub fn check_crlf(o: &str, e: &str, ins: &[Option<Insertion>]) -> Result<(), String> {
    let lines = o.split_inclusive('\n').zip(e.split_inclusive('\n'));
    for (i, ((o_line, e_line), ins)) in lines.zip(ins).enumerate() {
        if !o_line.ends_with("\r\n") {
            continue;
        }
        if !e_line.ends_with("\r\n") {
            return Err(format!("line {}: CRLF lost in {e_line:?}", i + 1));
        }
        if let Some(s) = ins
            && s.at > o_line.len() - 2
        {
            return Err(format!(
                "line {}: comment {:?} inside \\r\\n",
                i + 1,
                s.text
            ));
        }
    }
    Ok(())
}

/// P-comment-present (R2, C12): a line with a pinned component/include row
/// (`new` without `@sha256:`) and no `#` in the original gets an inserted
/// comment (insertions from [`check_byte_exact`]).
pub fn check_comments_present(
    o: &str,
    report: &Report,
    ins: &[Option<Insertion>],
) -> Result<(), String> {
    for (i, (o_line, ins)) in o.split_inclusive('\n').zip(ins).enumerate() {
        let needs = report.results.iter().any(|r| {
            r.line == i + 1
                && r.status == Status::Pinned
                && r.new.as_deref().is_some_and(|n| !n.contains("@sha256:"))
        });
        if needs && !o_line.contains('#') && ins.is_none() {
            return Err(format!(
                "line {}: pinned ref without comment: {o_line:?}",
                i + 1
            ));
        }
    }
    Ok(())
}

/// The findings the plugin plans for `text` (no file is read or written),
/// resolved against the fakes with the same policy [`pin_once`] uses.
pub async fn plan_once(f: &Fakes, name: &str, text: &str, mode: Mode) -> Vec<Finding> {
    let update = UpdatePolicy {
        enabled: matches!(mode, Mode::Update),
        ..Default::default()
    };
    let env = ResolverEnv::default();
    let ctx = Ctx::new(update, now_unix(), env)
        .with_oci(Ok(registry_client(f)))
        .with_gitlab("gitlab.com", Ok(gitlab_client(f)));
    GitlabCi
        .plan(
            &ctx,
            Path::new(name),
            text,
            &KindSet::defaults(&KINDS).into(),
            4,
        )
        .await
        .unwrap()
}

/// P-span (review L7): the edits of `findings` are tied to the spans
/// `discover(o)` reports, then must produce `e1`.
///
/// - an edited finding has exactly one replacement; it is exactly the span
///   of a discovered target on the finding's line, and `o[span] == old`;
/// - finding ↦ target is injective (no target is replaced twice);
/// - every insertion sits at the end of the finding's line, starts with
///   `"  # "` and has no `\n`;
/// - all of this is checked BEFORE `apply` (its `debug_assert` panics on
///   overlapping replacements, so an overlap must be an `Err` here);
/// - finally `apply(o, coalesce_comments(edits ordered by (line,
///   replacement start)))` equals `e1`: the combined comment is in byte
///   order whatever the scan order.
pub fn check_spans(o: &str, e1: &str, findings: &[Finding]) -> Result<(), String> {
    let targets = discover(o).expect("fixture parses");
    let lines = LineIndex::new(o);
    let mut claimed: Vec<Option<usize>> = vec![None; targets.len()];
    let mut ordered: Vec<(usize, usize, &Finding)> = Vec::new();
    for (i, f) in findings.iter().enumerate() {
        if f.edits.is_empty() {
            continue;
        }
        let at = format!("line {}", f.line + 1);
        let reps: Vec<&Splice> = f.edits.iter().filter(|s| s.start != s.end).collect();
        let [r] = reps[..] else {
            return Err(format!(
                "{at}: {} replacements, expected exactly one",
                reps.len()
            ));
        };
        let Some(ti) = targets
            .iter()
            .position(|t| t.line() == f.line && t.start() == r.start && t.end() == r.end)
        else {
            return Err(format!(
                "{at}: replacement {}..{} is not the span of a discovered target on this line",
                r.start, r.end
            ));
        };
        if let Some(prev) = claimed[ti].replace(i) {
            return Err(format!(
                "{at}: target {}..{} is replaced by two findings (#{prev} and #{i})",
                r.start, r.end
            ));
        }
        if o.get(r.start..r.end) != Some(f.old.as_str()) {
            return Err(format!(
                "{at}: replaced bytes {:?} differ from the finding's old {:?}",
                o.get(r.start..r.end),
                f.old
            ));
        }
        for s in f.edits.iter().filter(|s| s.start == s.end) {
            if lines.line_end(f.line) != Some(s.start) {
                return Err(format!(
                    "{at}: insertion at {} is not at the line end {:?}",
                    s.start,
                    lines.line_end(f.line)
                ));
            }
            if !s.text.starts_with("  # ") || s.text.contains('\n') {
                return Err(format!(
                    "{at}: insertion {:?} is not a one-line comment",
                    s.text
                ));
            }
        }
        ordered.push((f.line, r.start, f));
    }
    ordered.sort_by_key(|&(line, start, _)| (line, start));
    let edits: Vec<Splice> = ordered
        .iter()
        .flat_map(|(_, _, f)| f.edits.iter().cloned())
        .collect();
    let got = apply(o, &coalesce_comments(edits));
    if got != e1 {
        return Err(format!(
            "the file differs from the original with every replacement and the combined comments applied in byte order:\nexpected {got:?}\nactual   {e1:?}"
        ));
    }
    Ok(())
}
