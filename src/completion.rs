//! Shared planning and merging for the terminal host and diagnostic CLI.
use crate::{
    config::Config,
    engine::CommandIndex,
    model::{Candidate, CandidateKind, Completion, InputContext},
    ranking::{self, UsageSnapshot},
    sources::{SourceRequest, SourceUpdate},
    spec_catalog::Catalog,
};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
    sync::Arc,
};

#[allow(clippy::too_many_arguments)]
pub fn plan(
    index: &CommandIndex,
    catalog: &Catalog,
    line: &str,
    cursor: usize,
    cwd: &Path,
    context: Option<&InputContext>,
    revision: u64,
    environment: Arc<BTreeMap<String, String>>,
    fuzzy: bool,
    dynamic: bool,
    descriptions: &BTreeMap<String, String>,
) -> (Completion, SourceRequest) {
    let context = context
        .cloned()
        .unwrap_or_else(|| index.input_context(line, cursor));
    let result = index.complete_configured(
        line,
        cursor,
        cwd,
        usize::MAX,
        descriptions,
        Some(&context),
        fuzzy,
        false,
        Some(catalog),
    );
    let spec = (!context.command_position)
        .then(|| catalog.lookup_unfiltered(&context.command, &context.arguments, &context.prefix))
        .flatten();
    let provider = spec.as_ref().and_then(|s| s.provider.clone());
    let folded_prefix = context.prefix.to_ascii_lowercase();
    let expression = folded_prefix.starts_with("$env:") || folded_prefix.starts_with("${env:");
    let explicit_path = (context.prefix.contains(['/', '\\']) || context.prefix.starts_with('~'))
        && (context.command_position
            || spec
                .as_ref()
                .is_none_or(|s| s.path_values || s.options_ended));
    let paths = !context.suppressed
        && (expression
            || explicit_path
            || (!context.command_position
                && (context.command == "__shell_redirection"
                    || spec
                        .as_ref()
                        .is_none_or(|s| s.path_values || s.options_ended))))
        && (!context.prefix.starts_with('-') || spec.as_ref().is_some_and(|s| s.path_values));
    let directories_only = spec.as_ref().is_some_and(|s| s.directories_only)
        || matches!(
            context.command.to_ascii_lowercase().as_str(),
            "cd" | "set-location" | "push-location" | "pushd" | "chdir" | "sl"
        );
    let project = dynamic
        && !context.suppressed
        && !context.command_position
        && provider
            .as_ref()
            .is_some_and(|p| !p.starts_with("powershell."));
    (
        result,
        SourceRequest {
            revision,
            line: line.into(),
            cursor,
            context,
            cwd: cwd.into(),
            environment,
            paths,
            directories_only,
            project,
            provider,
            fuzzy,
        },
    )
}

pub fn merge(
    mut base: Completion,
    request: &SourceRequest,
    parts: [Option<&SourceUpdate>; 2],
    usage: Option<&UsageSnapshot>,
    limit: usize,
    descriptions: &BTreeMap<String, String>,
) -> Completion {
    for (index, part) in parts.into_iter().enumerate() {
        if let Some(part) = part {
            base.incomplete |= part.incomplete;
            base.candidates.extend(part.candidates.clone());
        } else {
            base.incomplete |= if index == 0 {
                request.project
            } else {
                request.paths
            };
        }
    }
    let mut paths = Vec::new();
    base.candidates.retain(|candidate| {
        if matches!(
            candidate.kind,
            CandidateKind::File | CandidateKind::Directory
        ) {
            paths.push(candidate.clone());
            false
        } else {
            true
        }
    });
    ranking::sort(
        &mut base.candidates,
        &request.context.prefix,
        request.fuzzy,
        usage,
        &request.cwd,
        usize::MAX,
    );
    // Directory candidates already match the last path segment, rather than
    // the full expression used by command and option matching.
    paths.sort_by_cached_key(|c| (c.kind != CandidateKind::Directory, c.label.to_lowercase()));
    base.candidates.extend(paths);
    let mut seen = HashSet::new();
    base.candidates
        .retain(|c| seen.insert((c.insert_text.clone(), c.kind.clone())));
    for candidate in &mut base.candidates {
        if let Some(description) = descriptions.get(candidate.identity()) {
            candidate.description = description.clone();
        }
        preserve_suffix(
            candidate,
            &request.line,
            request.cursor,
            base.replace_start,
            base.replace_end,
        );
    }
    base.candidates.truncate(limit);
    base
}

/// A completion may grow the part left of the cursor, but must never discard
/// unrelated text on its right. Consume the longest matching suffix overlap.
pub fn preserve_suffix(
    candidate: &mut Candidate,
    line: &str,
    cursor: usize,
    start: usize,
    end: usize,
) {
    if start > cursor || cursor >= end {
        return;
    }
    let Some(mut suffix) = line.get(cursor..end) else {
        return;
    };
    if suffix.is_empty() {
        return;
    }
    let quote = line
        .get(start..end)
        .and_then(|raw| raw.chars().next())
        .filter(|c| matches!(c, '\'' | '"'))
        .filter(|c| suffix.ends_with(*c) && candidate.insert_text.ends_with(*c));
    if let Some(quote) = quote {
        suffix = &suffix[..suffix.len() - quote.len_utf8()];
        candidate.insert_text.pop();
    }
    let mut overlap = 0;
    for (offset, _) in suffix
        .char_indices()
        .chain(std::iter::once((suffix.len(), '\0')))
    {
        if candidate.insert_text.ends_with(&suffix[..offset]) {
            overlap = offset;
        }
    }
    candidate.insert_text.push_str(&suffix[overlap..]);
    if let Some(quote) = quote {
        candidate.insert_text.push(quote);
    }
    candidate.append_space = false;
}

pub fn catalog_path(config: &Config, config_path: Option<&Path>) -> std::path::PathBuf {
    crate::config::specs_dir(config, config_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_parameters_filter_real_filesystem_results() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("sample-directory")).unwrap();
        std::fs::write(root.path().join("sample-file"), "fixture").unwrap();
        let index = CommandIndex::discover_with_env("".as_ref(), ".EXE".as_ref());
        for line in ["git -C sam", "npm --prefix sam", "pnpm --dir sam", "cd sam"] {
            let (base, request) = plan(
                &index,
                Catalog::builtin(),
                line,
                line.len(),
                root.path(),
                None,
                1,
                Arc::new(BTreeMap::new()),
                true,
                true,
                &BTreeMap::new(),
            );
            assert!(request.directories_only && request.paths, "{line}");
            let parts = crate::sources::collect_once(&request);
            let result = merge(
                base,
                &request,
                [Some(&parts[0]), Some(&parts[1])],
                None,
                100,
                &BTreeMap::new(),
            );
            assert!(
                result
                    .candidates
                    .iter()
                    .any(|c| c.label == "sample-directory"),
                "{line}"
            );
            assert!(
                result.candidates.iter().all(|c| c.label != "sample-file"),
                "{line}"
            );
        }
    }

    #[test]
    fn middle_insert_keeps_suffix_without_duplication() {
        for (line, cursor, value, expected) in [
            ("git", 1, "git", "git"),
            ("giXYZ", 2, "git", "gitXYZ"),
            ("'中文😀x'", 7, "'中文😀'", "'中文😀x'"),
            ("git tail", 2, "git", "git"),
        ] {
            let end = if line == "git tail" { 3 } else { line.len() };
            let mut c = Candidate {
                insert_text: value.into(),
                ..Default::default()
            };
            preserve_suffix(&mut c, line, cursor, 0, end);
            assert_eq!(c.insert_text, expected);
        }
    }

    #[test]
    fn manifest_values_flow_through_real_context_and_replacement_rules() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Cargo.toml"),"[package]\nname='demo'\nversion='0.1.0'\n[features]\nfast=[]\njson=[]\n[[bin]]\nname='worker'\n").unwrap();
        std::fs::write(root.path().join("package.json"),r#"{"name":"demo-js","scripts":{"dev":"echo unused","test:unit":"echo unused"},"dependencies":{"local-dep":"1.0.0"}}"#).unwrap();
        let index =
            CommandIndex::discover_with_env(std::ffi::OsStr::new(""), std::ffi::OsStr::new(".EXE"));
        for (line, wanted) in [
            ("cargo build --features j", "json"),
            ("cargo build --features=fast,j", "'--features=fast,json'"),
            ("cargo build --bin w", "worker"),
            ("pnpm run te", "test:unit"),
        ] {
            let (base, request) = plan(
                &index,
                Catalog::builtin(),
                line,
                line.len(),
                root.path(),
                None,
                1,
                Arc::new(BTreeMap::new()),
                true,
                true,
                &BTreeMap::new(),
            );
            assert!(request.project, "missing provider for {line}");
            let parts = crate::sources::collect_once(&request);
            let merged = merge(
                base,
                &request,
                [Some(&parts[0]), Some(&parts[1])],
                None,
                100,
                &BTreeMap::new(),
            );
            assert!(
                merged.candidates.iter().any(|c| c.insert_text == wanted),
                "{line}: {:?}; {:?}",
                merged.candidates,
                parts[0].diagnostics
            );
            assert!(!merged.incomplete, "{line}");
        }
    }
}
