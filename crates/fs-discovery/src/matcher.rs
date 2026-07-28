use std::path::Path;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::{CaseMode, DiscoveryError, DiscoverySelector, MatchMode};

pub(crate) struct CompiledSelector {
    patterns: Vec<String>,
    extensions: Vec<String>,
    match_mode: MatchMode,
    case_insensitive: bool,
    path_globs: Option<GlobSet>,
    name_globs: Option<GlobSet>,
}

impl CompiledSelector {
    pub(crate) fn new(selector: &DiscoverySelector) -> Result<Self, DiscoveryError> {
        let case_insensitive = case_insensitive(selector);
        let patterns = selector
            .patterns
            .iter()
            .map(|value| normalize_for_case(value, case_insensitive))
            .collect::<Vec<_>>();
        let extensions = selector
            .extensions
            .iter()
            .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let path_globs = compile_globs(&selector.globs, case_insensitive)?;
        let name_globs = (selector.match_mode == MatchMode::Glob)
            .then(|| compile_globs(&selector.patterns, case_insensitive))
            .transpose()?
            .flatten();
        Ok(Self {
            patterns,
            extensions,
            match_mode: selector.match_mode,
            case_insensitive,
            path_globs,
            name_globs,
        })
    }

    pub(crate) fn matches(&self, path: &Path, relative_to_root: &Path) -> bool {
        self.extension_matches(path)
            && self.pattern_matches(path)
            && self
                .path_globs
                .as_ref()
                .is_none_or(|globs| globs.is_match(normalized_path(relative_to_root)))
    }

    fn extension_matches(&self, path: &Path) -> bool {
        self.extensions.is_empty()
            || path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase())
                .is_some_and(|value| self.extensions.iter().any(|item| item == &value))
    }

    fn pattern_matches(&self, path: &Path) -> bool {
        if self.patterns.is_empty() {
            return true;
        }
        if let Some(globs) = &self.name_globs {
            return path.file_name().is_some_and(|name| globs.is_match(name));
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| normalize_for_case(value, self.case_insensitive))
            .unwrap_or_default();
        self.patterns.iter().any(|pattern| match self.match_mode {
            MatchMode::Exact => name == *pattern,
            MatchMode::StartsWith => name.starts_with(pattern),
            MatchMode::EndsWith => name.ends_with(pattern),
            MatchMode::Contains => {
                name.contains(pattern)
                    || pattern_stem(pattern).is_some_and(|stem| {
                        path.file_stem()
                            .and_then(|value| value.to_str())
                            .map(|value| normalize_for_case(value, self.case_insensitive))
                            .is_some_and(|value| value.contains(&stem))
                    })
            }
            MatchMode::Glob => false,
        })
    }
}

fn pattern_stem(pattern: &str) -> Option<String> {
    let path = Path::new(pattern);
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != pattern)
        .map(str::to_string)
}

fn compile_globs(
    patterns: &[String],
    case_insensitive: bool,
) -> Result<Option<GlobSet>, DiscoveryError> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .case_insensitive(case_insensitive)
            .literal_separator(true)
            .build()
            .map_err(|error| DiscoveryError::BackendFailed(format!("invalid_glob:{error}")))?;
        set.add(glob);
    }
    set.build()
        .map(Some)
        .map_err(|error| DiscoveryError::BackendFailed(format!("invalid_glob:{error}")))
}

fn case_insensitive(selector: &DiscoverySelector) -> bool {
    match selector.case_mode {
        CaseMode::Sensitive => false,
        CaseMode::Insensitive => true,
        CaseMode::Smart => !selector
            .patterns
            .iter()
            .chain(selector.globs.iter())
            .any(|value| value.chars().any(char::is_uppercase)),
    }
}

fn normalize_for_case(text: &str, case_insensitive: bool) -> String {
    let normalized = text
        .trim()
        .chars()
        .map(|ch| match ch {
            '／' | '＼' | '、' => '/',
            '－' => '-',
            '＿' => '_',
            '．' | '。' => '.',
            '（' => '(',
            '）' => ')',
            '【' => '[',
            '】' => ']',
            '｛' => '{',
            '｝' => '}',
            '　' => ' ',
            _ => ch,
        })
        .collect::<String>();
    if case_insensitive {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
