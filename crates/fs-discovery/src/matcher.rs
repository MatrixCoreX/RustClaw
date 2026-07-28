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
            MatchMode::Fuzzy => {
                fuzzy_score(&name, pattern).is_some()
                    || path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .map(|value| normalize_for_case(value, self.case_insensitive))
                        .is_some_and(|stem| fuzzy_score(&stem, pattern).is_some())
            }
            MatchMode::Glob => false,
        })
    }
}

pub fn fuzzy_name_score(candidate: &str, pattern: &str, case_mode: CaseMode) -> Option<usize> {
    let case_insensitive = match case_mode {
        CaseMode::Sensitive => false,
        CaseMode::Insensitive => true,
        CaseMode::Smart => !pattern.chars().any(char::is_uppercase),
    };
    let candidate = normalize_for_case(candidate, case_insensitive);
    let pattern = normalize_for_case(pattern, case_insensitive);
    let full = fuzzy_score(&candidate, &pattern);
    let stem = Path::new(&candidate)
        .file_stem()
        .and_then(|value| value.to_str())
        .and_then(|value| fuzzy_score(value, &pattern));
    full.into_iter().chain(stem).min()
}

fn fuzzy_score(candidate: &str, pattern: &str) -> Option<usize> {
    if candidate.is_empty() || pattern.is_empty() {
        return None;
    }
    if candidate == pattern {
        return Some(0);
    }
    if candidate.starts_with(pattern) {
        return Some(
            4 + candidate
                .chars()
                .count()
                .saturating_sub(pattern.chars().count()),
        );
    }
    if let Some(position) = candidate.find(pattern) {
        return Some(16 + position);
    }

    let candidate_chars = candidate.chars().collect::<Vec<_>>();
    let pattern_chars = pattern.chars().collect::<Vec<_>>();
    let allowed_distance = match pattern_chars.len() {
        0..=2 => 0,
        3..=5 => 1,
        6..=9 => 2,
        length => (length / 4).min(4),
    };
    if allowed_distance > 0 {
        if let Some(distance) =
            bounded_damerau_levenshtein(&candidate_chars, &pattern_chars, allowed_distance)
        {
            return Some(64 + distance * 16 + candidate_chars.len().abs_diff(pattern_chars.len()));
        }
    }

    if pattern_chars.len() >= 3
        && candidate_chars.len() <= pattern_chars.len().saturating_mul(4).saturating_add(8)
    {
        subsequence_gap_score(&candidate_chars, &pattern_chars).map(|gaps| 160 + gaps)
    } else {
        None
    }
}

fn bounded_damerau_levenshtein(
    candidate: &[char],
    pattern: &[char],
    max_distance: usize,
) -> Option<usize> {
    if candidate.len().abs_diff(pattern.len()) > max_distance {
        return None;
    }
    let mut previous_previous = vec![0usize; pattern.len() + 1];
    let mut previous = (0..=pattern.len()).collect::<Vec<_>>();
    let mut current = vec![0usize; pattern.len() + 1];
    for (candidate_index, candidate_char) in candidate.iter().enumerate() {
        current[0] = candidate_index + 1;
        let mut row_minimum = current[0];
        for (pattern_index, pattern_char) in pattern.iter().enumerate() {
            let substitution =
                previous[pattern_index] + usize::from(candidate_char != pattern_char);
            let insertion = current[pattern_index] + 1;
            let deletion = previous[pattern_index + 1] + 1;
            let mut distance = substitution.min(insertion).min(deletion);
            if candidate_index > 0
                && pattern_index > 0
                && candidate[candidate_index] == pattern[pattern_index - 1]
                && candidate[candidate_index - 1] == pattern[pattern_index]
            {
                distance = distance.min(previous_previous[pattern_index - 1] + 1);
            }
            current[pattern_index + 1] = distance;
            row_minimum = row_minimum.min(distance);
        }
        if row_minimum > max_distance {
            return None;
        }
        std::mem::swap(&mut previous_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    (previous[pattern.len()] <= max_distance).then_some(previous[pattern.len()])
}

fn subsequence_gap_score(candidate: &[char], pattern: &[char]) -> Option<usize> {
    let mut pattern_index = 0usize;
    let mut first_match = None;
    for (candidate_index, candidate_char) in candidate.iter().enumerate() {
        if pattern.get(pattern_index) == Some(candidate_char) {
            first_match.get_or_insert(candidate_index);
            pattern_index += 1;
            if pattern_index == pattern.len() {
                let span = candidate_index.saturating_sub(first_match.unwrap_or(0)) + 1;
                return Some(span.saturating_sub(pattern.len()) + first_match.unwrap_or(0));
            }
        }
    }
    None
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
