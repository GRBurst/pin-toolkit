//! Discovery of container image, component and classic include references
//! in GitLab CI files: path matching over `pin_core::yaml` (M5c-y).
//!
//! [`discover`] scans the file once (`yaml::scan`, no template masking —
//! GitLab CI is not Go-templated) and matches each scalar's path, relative
//! to its document root:
//!
//! - `image`: `image` / `services` at the root or one key down (a job,
//!   `default`, a hidden template), followed by nothing, `name`, an index,
//!   or an index and `name`;
//! - `component`: exactly the root key `component`; `parts` is `None` when
//!   the value fails `parse_component_ref`;
//! - `include`: a mapping at `include` or `include[n]` (root only) with a
//!   `project` and a `ref` scalar, grouped by mapping (first of each, any
//!   order); the target is the `ref` value.
//!
//! An implicit null has an empty span and never matches. A block scalar or
//! a scalar whose text spans lines is [`PinTarget::Unsupported`]. Aliases,
//! merge keys and `!reference` nodes are not scalars: never targets. A file
//! that does not parse is a [`ScanError`] (no fallback, D32).

use std::collections::BTreeMap;

use pin_core::plugin::ScanError;
use pin_core::yaml::{self, Node, PathSeg, ScalarRef, ScanOpts, Style};

/// One discovered reference location in a CI file (M3: image, component or
/// classic include).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinTarget {
    /// A container image reference (`image:` / `services:` value).
    Image {
        /// 0-based line index in the file text.
        line: usize,
        /// Byte offset where the value token begins (incl. quotes).
        start: usize,
        /// Byte offset where the value token ends (excl. comments).
        end: usize,
        /// The value with quotes stripped.
        value: String,
    },
    /// A top-level GitLab component reference (`component:` value).
    Component {
        /// 0-based line index in the file text.
        line: usize,
        /// Byte offset where the value token begins (incl. quotes).
        start: usize,
        /// Byte offset where the value token ends (excl. comments).
        end: usize,
        /// The value with quotes stripped.
        value: String,
        /// Parsed `host / project / name @ version` pieces, or `None` when
        /// the value is unparseable (reported `skipped` at resolve time —
        /// the target is kept so the report stays complete).
        parts: Option<ComponentParts>,
    },
    /// A classic top-level (indent 0) `include:` mapping with explicit
    /// `project` and `ref` keys: the span points at the `ref` VALUE token
    /// (the value to be replaced with a commit SHA), not the mapping.
    IncludeRef {
        /// 0-based line index of the `ref` value token in the file text.
        line: usize,
        /// Byte offset where the `ref` value token begins (incl. quotes).
        start: usize,
        /// Byte offset where the `ref` value token ends (excl. comments).
        end: usize,
        /// The `ref` value, quotes stripped.
        value: String,
        /// The `project` path, quotes stripped.
        project: String,
        /// True when the mapping is a flow mapping
        /// (`{ project: ..., ref: ... }`).
        flow: bool,
    },
    /// A reference at a target path that cannot be spliced (a block or
    /// multi-line scalar): reported `skipped (<reason>)`, never edited.
    ///
    /// `reason` is `block scalar` or `multi-line scalar`; `start..end` is
    /// one line (the range is cut at the first line break).
    Unsupported {
        /// The kind the reference would have: `image`, `component` or
        /// `include`.
        kind: &'static str,
        /// 0-based line index in the file text.
        line: usize,
        /// Byte offset where the value begins.
        start: usize,
        /// Byte offset where the (one-line) range ends.
        end: usize,
        /// The decoded value.
        value: String,
        /// Why it is skipped.
        reason: &'static str,
    },
}

/// Parsed pieces of a component reference
/// `host/group[/group…]/name@version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentParts {
    /// The GitLab instance host (e.g. `gitlab.com`).
    pub host: String,
    /// The project path (e.g. `group/sub/project`).
    pub project: String,
    /// The component name (last path segment).
    pub name: String,
    /// The version token after the last `@`.
    pub version: String,
}

impl PinTarget {
    /// 0-based line index in the file text.
    pub fn line(&self) -> usize {
        match self {
            Self::Image { line, .. }
            | Self::Component { line, .. }
            | Self::IncludeRef { line, .. }
            | Self::Unsupported { line, .. } => *line,
        }
    }

    /// Byte offset where the value token begins (incl. quotes).
    pub fn start(&self) -> usize {
        match self {
            Self::Image { start, .. }
            | Self::Component { start, .. }
            | Self::IncludeRef { start, .. }
            | Self::Unsupported { start, .. } => *start,
        }
    }

    /// Byte offset where the value token ends (excl. comments).
    pub fn end(&self) -> usize {
        match self {
            Self::Image { end, .. }
            | Self::Component { end, .. }
            | Self::IncludeRef { end, .. }
            | Self::Unsupported { end, .. } => *end,
        }
    }

    /// The value with quotes stripped.
    ///
    /// For `IncludeRef` this is the `ref` value (not `project`).
    pub fn value(&self) -> &str {
        match self {
            Self::Image { value, .. }
            | Self::Component { value, .. }
            | Self::IncludeRef { value, .. }
            | Self::Unsupported { value, .. } => value,
        }
    }
}

/// Discovery over `pin_core::yaml` (M5c-y): path rules over the value nodes
/// of [`yaml::scan`]. Returns targets sorted by `(line, start)`;
/// `Err` = the file is not valid YAML (§6.9).
///
/// A scalar with a non-empty span (an implicit null has an empty one) is a
/// target when its path, relative to its document root, is
/// - `image`: `image` / `services` (at the root or one key down: a job),
///   followed by nothing, `name`, an index, or an index and `name`;
/// - `component`: exactly `component` (root only);
/// - `include`: a mapping at `include` or `include[n]` (root only) that has
///   a `project` and a `ref` scalar (first of each, any order, grouped by
///   mapping): the target is the `ref`.
///
/// A block scalar, or a scalar whose text spans lines, is not spliceable: it
/// becomes [`PinTarget::Unsupported`] (range cut at the first line break).
/// Aliases, merge keys and `!reference` nodes are not scalars here, so they
/// are never targets.
pub fn discover(text: &str) -> Result<Vec<PinTarget>, ScanError> {
    let nodes = yaml::scan(
        text,
        ScanOpts {
            mask_templates: false,
        },
    )?;
    let mut out = Vec::new();
    // include mappings by their collection id: (first `project`, first `ref`)
    let mut includes: BTreeMap<usize, (Option<&ScalarRef>, Option<&ScalarRef>)> = BTreeMap::new();
    for node in &nodes {
        let Node::Scalar(s) = node else { continue };
        if s.span.is_empty() {
            continue;
        }
        if is_image_path(&s.path) {
            out.push(yaml_target(text, s, "image"));
        } else if is_component_path(&s.path) {
            out.push(yaml_target(text, s, "component"));
        } else if let [m @ .., PathSeg::Key(k)] = &s.path[..]
            && is_include_mapping(m)
        {
            let entry = includes.entry(s.parent).or_default();
            match k.as_str() {
                "project" => entry.0 = entry.0.or(Some(s)),
                "ref" => entry.1 = entry.1.or(Some(s)),
                _ => {}
            }
        }
    }
    for (project, git_ref) in includes.into_values() {
        let (Some(project), Some(git_ref)) = (project, git_ref) else {
            continue;
        };
        out.push(
            unsupported(text, git_ref, "include").unwrap_or_else(|| PinTarget::IncludeRef {
                line: git_ref.line,
                start: git_ref.span.start,
                end: git_ref.span.end,
                value: git_ref.value.clone(),
                project: project.value.clone(),
                flow: git_ref.in_flow,
            }),
        );
    }
    out.sort_by_key(|t| (t.line(), t.start()));
    Ok(out)
}

/// `Unsupported` when `s` cannot be spliced (a block scalar, or text that
/// spans lines); its range is cut at the first line break.
fn unsupported(text: &str, s: &ScalarRef, kind: &'static str) -> Option<PinTarget> {
    let token = &text[s.span.clone()];
    let reason = if s.style == Style::Block {
        "block scalar"
    } else if token.contains('\n') {
        "multi-line scalar"
    } else {
        return None;
    };
    Some(PinTarget::Unsupported {
        kind,
        line: s.line,
        start: s.span.start,
        end: s.span.start + token.find(['\r', '\n']).unwrap_or(token.len()),
        value: s.value.clone(),
        reason,
    })
}

/// The `image` or `component` target for `s`.
fn yaml_target(text: &str, s: &ScalarRef, kind: &'static str) -> PinTarget {
    if let Some(u) = unsupported(text, s, kind) {
        return u;
    }
    let (line, start, end, value) = (s.line, s.span.start, s.span.end, s.value.clone());
    if kind == "component" {
        PinTarget::Component {
            line,
            start,
            end,
            parts: parse_component_ref(&value),
            value,
        }
    } else {
        PinTarget::Image {
            line,
            start,
            end,
            value,
        }
    }
}

/// `q` is `image|services` followed by nothing, `name`, an index, or an
/// index and `name`.
fn image_tail(q: &[PathSeg]) -> bool {
    let [PathSeg::Key(head), rest @ ..] = q else {
        return false;
    };
    if head != "image" && head != "services" {
        return false;
    }
    match rest {
        [] | [PathSeg::Index(_)] => true,
        [PathSeg::Key(n)] | [PathSeg::Index(_), PathSeg::Key(n)] => n == "name",
        _ => false,
    }
}

/// `image` / `services` at the root, or one key down (a job, `default`, a
/// hidden template).
fn is_image_path(p: &[PathSeg]) -> bool {
    image_tail(p) || (matches!(p.first(), Some(PathSeg::Key(_))) && image_tail(&p[1..]))
}

/// The mapping of a classic include: the root key `include` itself, or one
/// item of its sequence.
fn is_include_mapping(p: &[PathSeg]) -> bool {
    match p {
        [PathSeg::Key(k)] | [PathSeg::Key(k), PathSeg::Index(_)] => k == "include",
        _ => false,
    }
}

/// Exactly the root key `component`.
fn is_component_path(p: &[PathSeg]) -> bool {
    matches!(p, [PathSeg::Key(k)] if k == "component")
}

/// Split a component reference `host/group[/group…]/name@version` into
/// pieces.
///
/// The split is at the LAST `@`. The prefix must have at least 3 non-empty
/// '/'-separated segments; the first (host) must contain a `.` and no `$`;
/// the version (after the last `@`) must be non-empty. Anything else is
/// `None` (the caller keeps the target and reports it `skipped` at resolve
/// time).
fn parse_component_ref(value: &str) -> Option<ComponentParts> {
    let (prefix, version) = value.rsplit_once('@')?;
    if version.is_empty() {
        return None;
    }
    let segments: Vec<&str> = prefix.split('/').collect();
    if segments.len() < 3 || segments.iter().any(|s| s.is_empty()) {
        return None;
    }
    let host = segments[0];
    if !host.contains('.') || host.contains('$') {
        return None;
    }
    Some(ComponentParts {
        host: host.to_string(),
        project: segments[1..segments.len() - 1].join("/"),
        name: segments[segments.len() - 1].to_string(),
        version: version.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = include_str!("../tests/fixtures/ci-simple.yml");
    const RICH: &str = include_str!("../tests/fixtures/ci-rich.yml");

    const INCLUDE_FIXTURE: &str = include_str!("../tests/fixtures/ci-include.yml");

    #[test]
    fn image_targets_survive_enum_migration() {
        // M1/M2 image expectations, unchanged: ci-simple.yml …
        let t = discover(SIMPLE).unwrap();
        let got: Vec<(usize, &str)> = t.iter().map(|x| (x.line(), x.value())).collect();
        assert_eq!(
            got,
            vec![
                (1, "alpine:3.20"),  // top-level image
                (4, "ubuntu:24.04"), // default.image
                (7, "node:20"),      // build.image (quoted; value unquoted)
                (8, "redis:7"),      // build.services scalar
                (11, "alpine:3.20"), // test.image (same image twice)
            ]
        );
        // spans must cover the exact written token (quotes included in span, not value)
        for x in &t {
            let token = &SIMPLE[x.start()..x.end()];
            let unquoted = token.trim_matches(|c| c == '"' || c == '\'');
            assert_eq!(unquoted, x.value());
        }
        // … and ci-rich.yml.
        let t = discover(RICH).unwrap();
        let got: Vec<(usize, &str)> = t
            .iter()
            .filter_map(|x| match x {
                PinTarget::Image { .. } => Some((x.line(), x.value())),
                // M3 include targets in this fixture are asserted separately.
                _ => None,
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (2, "alpine:3.20"), // default.image with trailing comment
                (3, "docker:24"),   // flow item 1
                (3, "redis:7"),     // flow item 2 (same line, later span)
                (7, "node:20"),     // image mapping name
                (10, "docker:24"),  // sequence scalar
                (11, "redis:7"),    // sequence mapping name
            ]
        );
    }

    #[test]
    fn templated_value_is_still_a_target() {
        // Shell/YAML interpolation is a syntactic image candidate; the parse
        // rejection happens at resolve time (reported as `skipped
        // (unparseable reference)`), not here — discover must not silently
        // drop lines the user should see in the report.
        let text = "job:\n  image: ${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}\n";
        let t = discover(text).unwrap();
        assert_eq!(t.len(), 1, "templated value must stay a target");
        assert_eq!(t[0].value(), "${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}");
    }

    // ── M3 Task 2: classic include: discovery ───────────────────────

    #[test]
    fn include_flow_single_mapping() {
        // { project: x, ref: y } → target, flow=true, span on the ref value
        let text = "include: { project: gitlab-org/example, ref: main }  # flow single mapping\n";
        let t = discover(text).unwrap();
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                line,
                start,
                end,
                value,
                project,
                flow,
            } => {
                assert_eq!(value, "main");
                assert_eq!(project, "gitlab-org/example");
                assert_eq!(&text[*start..*end], "main");
                assert_eq!(*line, 0);
                assert!(*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_flow_sequence_mixed() {
        // [ {p+ref}, "str", {p only} ] → exactly 1 target (the flow mapping item)
        let text = "include: [ { project: g/p, ref: main }, \"local.yml\", { project: g/q } ]\n";
        let t = discover(text).unwrap();
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                value,
                project,
                flow,
                ..
            } => {
                assert_eq!(value, "main");
                assert_eq!(project, "g/p");
                assert!(*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_block_sequence_item() {
        // - project: x / ref: y (+file:) → target on the ref value, flow=false
        let text =
            "include:\n  - project: gitlab-org/example2\n    ref: v1.2.3\n    file: /ci/test.yml\n";
        let t = discover(text).unwrap();
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                line,
                start,
                end,
                value,
                project,
                flow,
            } => {
                assert_eq!(value, "v1.2.3");
                assert_eq!(project, "gitlab-org/example2");
                assert_eq!(&text[*start..*end], "v1.2.3");
                assert_eq!(*line, 2);
                assert!(!*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_block_single_mapping() {
        // ref before project → still a target; span on the ref line
        let text = "include:\n  ref: develop\n  project: gitlab-org/single\n";
        let t = discover(text).unwrap();
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                line,
                start,
                end,
                value,
                project,
                flow,
            } => {
                assert_eq!(value, "develop");
                assert_eq!(project, "gitlab-org/single");
                assert_eq!(&text[*start..*end], "develop");
                assert_eq!(*line, 1);
                assert!(!*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_string_and_remote_not_targets() {
        // "x.yml", remote:, template:, local: items → no IncludeRef at all
        let text = "include:\n  - \"x.yml\"\n  - remote: https://example.com/ci.yml\n  - template: something.yml\n  - local: /x.yml\n";
        assert!(discover(text).unwrap().is_empty());
    }

    #[test]
    fn include_indented_not_a_target() {
        // Job-internal includes (indented) are out of scope: indent-0 only
        let text = "job:\n  include: { project: gitlab-org/indented, ref: main }\n";
        assert!(discover(text).unwrap().is_empty());
    }

    #[test]
    fn include_templated_ref_still_a_target() {
        // ref: ${CI_REF} → still a target (syntactic discovery, no expansion)
        let text = "include:\n  project: g/p\n  ref: ${CI_REF}\n";
        let t = discover(text).unwrap();
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef { value, project, .. } => {
                assert_eq!(value, "${CI_REF}");
                assert_eq!(project, "g/p");
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_quoted_values() {
        // Quoted project/ref → target; span includes the quotes, value does not
        let text = "include:\n  project: \"g/p\"\n  ref: 'main'\n";
        let t = discover(text).unwrap();
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                line,
                start,
                end,
                value,
                project,
                flow,
            } => {
                assert_eq!(value, "main");
                assert_eq!(project, "g/p");
                assert_eq!(&text[*start..*end], "'main'");
                assert_eq!(*line, 2);
                assert!(!*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn combined_targets_sorted_by_line_then_start() {
        // image + component + include in one text: one global (line, start)
        // sort across all three kinds
        let text = "image: alpine:3.20\ncomponent: gitlab.com/g/p/name@1.0.0\ninclude: { project: g/p, ref: main }\ninclude:\n  project: g/p2\n  ref: v9\n";
        let t = discover(text).unwrap();
        let got: Vec<(usize, usize, &str)> = t
            .iter()
            .map(|x| match x {
                PinTarget::Image { .. } => (x.line(), x.start(), "image"),
                PinTarget::Component { .. } => (x.line(), x.start(), "component"),
                PinTarget::IncludeRef { .. } => (x.line(), x.start(), "include"),
                PinTarget::Unsupported { .. } => unreachable!("not produced"),
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (0, 7, "image"),
                (1, 30, "component"), // line off 19 + 11
                (2, 86, "include"),   // line off 56 + 30
                (5, 125, "include"),  // line off 118 + 7
            ]
        );
        assert_eq!(&text[t[2].start()..t[2].end()], "main");
        assert_eq!(&text[t[3].start()..t[3].end()], "v9");
    }

    #[test]
    fn include_fixture_all_forms() {
        // The fixture exercises every documented form: exactly the three
        // annotated targets, nothing else (indented include excluded).
        let t = discover(INCLUDE_FIXTURE).unwrap();
        assert_eq!(t.len(), 3);
        let got: Vec<(usize, &str, &str, bool)> = t
            .iter()
            .map(|x| match x {
                PinTarget::IncludeRef { project, flow, .. } => {
                    (x.line(), x.value(), project.as_str(), *flow)
                }
                other => panic!("unexpected target: {other:?}"),
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (0, "main", "gitlab-org/example", true),
                (3, "v1.2.3", "gitlab-org/example2", false),
                (10, "develop", "gitlab-org/single", false),
            ]
        );
    }

    #[test]
    fn rich_include_block_is_include_target() {
        // M3: ci-rich.yml's include block (project + ref) is now a target
        let t = discover(RICH).unwrap();
        let got: Vec<(usize, &str, &str)> = t
            .iter()
            .filter_map(|x| match x {
                PinTarget::IncludeRef { project, .. } => {
                    Some((x.line(), x.value(), project.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(got, vec![(16, "v1.0.0", "group/proj")]);
    }

    #[test]
    fn script_items_are_not_targets() {
        let t = discover(RICH).unwrap();
        assert!(t.iter().all(|x| x.value() != "echo hi"));
    }

    #[test]
    fn trailing_comment_preserved_in_spans() {
        let t = discover(RICH).unwrap();
        let first = &t[0];
        assert_eq!(&RICH[first.start()..first.end()], "alpine:3.20");
    }

    #[test]
    fn component_top_level_target() {
        let text = "component: gitlab.com/g/sub/p/name@1.0.0\n";
        let t = discover(text).unwrap();
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::Component {
                value,
                parts,
                start,
                end,
                line,
            } => {
                assert_eq!(value, "gitlab.com/g/sub/p/name@1.0.0");
                let p = parts.as_ref().expect("parts");
                assert_eq!(
                    (
                        p.host.as_str(),
                        p.project.as_str(),
                        p.name.as_str(),
                        p.version.as_str()
                    ),
                    ("gitlab.com", "g/sub/p", "name", "1.0.0")
                );
                assert_eq!(&text[*start..*end], "gitlab.com/g/sub/p/name@1.0.0");
                assert_eq!(*line, 0);
            }
            other => panic!("expected Component, got {other:?}"),
        }
    }

    #[test]
    fn component_indented_is_not_a_target() {
        let text = "job:\n  component: gitlab.com/g/p/name@1.0.0\n";
        assert!(discover(text).unwrap().is_empty());
    }

    #[test]
    fn component_unparseable_kept_with_none_parts() {
        for v in [
            "gitlab.com/name@1.0.0", // no project segment
            "gitlab.com/g/p/name",   // no '@'
            "${VAR}@1.0.0",          // templated host
            "gitlab.com/g/p/name@",  // empty version
        ] {
            let text = format!("component: {v}\n");
            let t = discover(&text).unwrap();
            assert_eq!(t.len(), 1, "value {v} must stay a target");
            match &t[0] {
                PinTarget::Component { parts, value, .. } => {
                    assert!(parts.is_none(), "value {v} must have no parts");
                    assert_eq!(value, v);
                }
                other => panic!("expected Component, got {other:?}"),
            }
        }
    }

    #[test]
    fn component_quoted_and_comment() {
        let text = "component: \"gitlab.com/g/p/name@2.0\"  # from docs\n";
        let t = discover(text).unwrap();
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::Component {
                value, start, end, ..
            } => {
                assert_eq!(value, "gitlab.com/g/p/name@2.0");
                assert_eq!(&text[*start..*end], "\"gitlab.com/g/p/name@2.0\"");
            }
            other => panic!("expected Component, got {other:?}"),
        }
    }

    #[test]
    #[allow(clippy::type_complexity)] // (input, Option<(host, project, name, version)>) table
    fn parse_component_ref_table() {
        let cases: [(&str, Option<(&str, &str, &str, &str)>); 6] = [
            (
                "gitlab.com/g/p/name@1.0.0",
                Some(("gitlab.com", "g/p", "name", "1.0.0")),
            ),
            (
                "gitlab.com/a/b/c/d/name@v9",
                Some(("gitlab.com", "a/b/c/d", "name", "v9")),
            ),
            ("gitlab.com/name@1.0.0", None), // no project segment
            ("gitlab.com/g/p/name", None),   // no '@'
            ("gitlab.com/g/p/name@", None),  // empty version
            ("$CI_SERVER_FQDN/g/p/name@1.0.0", None), // templated host
        ];
        for (input, expected) in cases {
            let parsed = parse_component_ref(input);
            let got = parsed.as_ref().map(|p| {
                (
                    p.host.as_str(),
                    p.project.as_str(),
                    p.name.as_str(),
                    p.version.as_str(),
                )
            });
            assert_eq!(got, expected, "parse_component_ref({input})");
        }
    }

    // ---- M5c-y: discovery over `core::yaml` (`discover`)

    fn yaml(text: &str) -> Vec<PinTarget> {
        discover(text).unwrap()
    }

    #[test]
    fn yaml_image_paths() {
        let text = "image: a:1\nservices: [b:1, {name: c:1}]\ndefault:\n  image: {name: d:1, entrypoint: [\"\"]}\njob:\n  services:\n    - e:1\n    - name: f:1\n      alias: g\n  script: [h:1]\n  deep:\n    image: i:1\nbäckend:\n  image: j:1\n\"quoted\":\n  image: k:1\nflowjob: {image: l:1}\n";
        let t = yaml(text);
        let values: Vec<&str> = t.iter().map(PinTarget::value).collect();
        assert_eq!(
            values,
            [
                "a:1", "b:1", "c:1", "d:1", "e:1", "f:1", "j:1", "k:1", "l:1"
            ]
        );
        assert!(t.iter().all(|x| matches!(x, PinTarget::Image { .. })));
        assert!(t.iter().all(|x| &text[x.start()..x.end()] == x.value()));
    }

    #[test]
    fn yaml_component_root_only() {
        let text = "component: gitlab.com/g/p/n@1.0.0\njob:\n  component: gitlab.com/g/p/n@2.0.0\n";
        let t = yaml(text);
        assert_eq!(t.len(), 1, "{t:?}");
        let PinTarget::Component { parts, value, .. } = &t[0] else {
            panic!("{:?}", t[0])
        };
        assert_eq!(value, "gitlab.com/g/p/n@1.0.0");
        let p = parts.as_ref().unwrap();
        assert_eq!(
            (&*p.host, &*p.project, &*p.name, &*p.version),
            ("gitlab.com", "g/p", "n", "1.0.0")
        );
    }

    #[test]
    fn yaml_block_scalar_is_unsupported() {
        let t = yaml("image: >-\n  alpine:3.20\ncomponent: |\n  gitlab.com/g/p/n@1.0.0\n");
        let got: Vec<(&str, usize, &str)> = t
            .iter()
            .map(|x| match x {
                PinTarget::Unsupported {
                    kind, line, reason, ..
                } => (*kind, *line, *reason),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            got,
            [
                ("image", 1, "block scalar"),
                ("component", 3, "block scalar")
            ]
        );

        // several content lines: the range is the first line only
        let text = "image: |\n  a:1\n  b:1\nnext: x\n";
        let t = yaml(text);
        assert_eq!(t.len(), 1, "{t:?}");
        assert!(matches!(
            &t[0],
            PinTarget::Unsupported {
                reason: "block scalar",
                ..
            }
        ));
        assert_eq!(&text[t[0].start()..t[0].end()], "a:1");
    }

    #[test]
    fn yaml_aliases_and_references_are_not_targets() {
        let text = ".x: &x alpine:3.20\na:\n  image: *x\nb:\n  image: !reference [.base, image]\n  services: !reference [.base, services]\n";
        assert_eq!(discover(text), Ok(vec![]));
    }

    /// Review M3: a quoted or plain value continued on the next line is
    /// never spliced; the range is cut at the first line break.
    #[test]
    fn yaml_multiline_scalar_is_unsupported() {
        let cases: [(&str, &str, &str, &str); 4] = [
            (
                "image: \"alpine:\\\n  3.20\"\nnext: x\n",
                "image",
                "alpine:3.20",
                "\"alpine:\\",
            ),
            (
                "image: alpine\n  :3.20\nnext: x\n",
                "image",
                "alpine :3.20",
                "alpine",
            ),
            (
                "image: \"alpine:\\\r\n  3.20\"\r\nnext: x\r\n",
                "image",
                "alpine:3.20",
                "\"alpine:\\",
            ),
            (
                "component: \"gitlab.com/g/p/n\\\n  @1.0\"\n",
                "component",
                "gitlab.com/g/p/n@1.0",
                "\"gitlab.com/g/p/n\\",
            ),
        ];
        for (text, kind, value, old) in cases {
            let t = yaml(text);
            assert_eq!(t.len(), 1, "{text:?}: {t:?}");
            let PinTarget::Unsupported {
                kind: k,
                value: v,
                reason,
                ..
            } = &t[0]
            else {
                panic!("{text:?}: {:?}", t[0])
            };
            assert_eq!(
                (*k, v.as_str(), *reason),
                (kind, value, "multi-line scalar")
            );
            assert_eq!(&text[t[0].start()..t[0].end()], old, "{text:?}");
        }
        let crlf = yaml("image: \"alpine:\\\r\n  3.20\"\r\nnext: x\r\n");
        assert_eq!((crlf[0].start(), crlf[0].end()), (7, 16));
    }

    /// Include mappings (`project` + `ref`) in every written form.
    #[test]
    fn yaml_include_forms() {
        // (line, value, project, flow)
        type Hit = (usize, &'static str, &'static str, bool);
        let cases: [(&str, Vec<Hit>); 12] = [
            (
                "include: { project: g/p, ref: main }\n",
                vec![(0, "main", "g/p", true)],
            ),
            (
                "include: [ {project: g/p, ref: main}, \"x.yml\", {project: g/q} ]\n",
                vec![(0, "main", "g/p", true)],
            ),
            (
                "include:\n  - project: g/p\n    ref: main\n    file: a.yml\n    inputs:\n      a: 1\n",
                vec![(2, "main", "g/p", false)],
            ),
            (
                "include:\n  ref: main\n  project: g/p\n",
                vec![(1, "main", "g/p", false)],
            ),
            (
                "include: { project: g/a, ref: x }\ninclude: { project: g/b, ref: y }\n",
                vec![(0, "x", "g/a", true), (1, "y", "g/b", true)],
            ),
            ("include:\n  project: g/p\n  ref:\n", vec![]),
            ("include: {project: p, ref}\n", vec![]),
            ("include: {ref, project: p}\n", vec![]),
            ("job:\n  include: {project: g/p, ref: main}\n", vec![]),
            ("include:\n  - component: gitlab.com/g/p/c@1.0\n", vec![]),
            (
                "include: [{project: g/p, ref: main, inputs: {a: 1}}]\n",
                vec![(0, "main", "g/p", true)],
            ),
            (
                "include:\n  project: \"g/p\"\n  ref: 'main'\n",
                vec![(2, "main", "g/p", false)],
            ),
        ];
        for (text, want) in cases {
            let found = yaml(text);
            let got: Vec<(usize, &str, &str, bool)> = found
                .iter()
                .map(|t| match t {
                    PinTarget::IncludeRef {
                        line,
                        value,
                        project,
                        flow,
                        ..
                    } => (*line, value.as_str(), project.as_str(), *flow),
                    other => panic!("{text:?}: {other:?}"),
                })
                .collect();
            assert_eq!(got, want, "{text:?}");
        }
        // the span of a quoted ref is the quoted token
        let text = "include:\n  project: \"g/p\"\n  ref: 'main'\n";
        let t = yaml(text);
        assert_eq!(&text[t[0].start()..t[0].end()], "'main'");

        // a multi-line quoted ref and a block ref are not spliceable
        let text = "include: { project: g/p, ref: \"ma\\\n  in\" }\n";
        let t = yaml(text);
        assert_eq!(t.len(), 1, "{t:?}");
        assert!(matches!(
            &t[0],
            PinTarget::Unsupported {
                kind: "include",
                reason: "multi-line scalar",
                ..
            }
        ));
        assert_eq!(&text[t[0].start()..t[0].end()], "\"ma\\");
        let t = yaml("include:\n  project: g/p\n  ref: >-\n    main\n");
        assert_eq!(t.len(), 1, "{t:?}");
        assert!(matches!(
            &t[0],
            PinTarget::Unsupported {
                kind: "include",
                reason: "block scalar",
                ..
            }
        ));
    }
}
