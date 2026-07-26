use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Helper};

use crate::chat_command::command_specs;

#[derive(Default)]
pub(crate) struct ChatEditorHelper;

impl Helper for ChatEditorHelper {}

impl Completer for ChatEditorHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        position: usize,
        _context: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let prefix = &line[..position];
        if prefix.starts_with('/') && !prefix.contains(char::is_whitespace) {
            return Ok((
                0,
                command_specs()
                    .iter()
                    .filter(|spec| spec.token.starts_with(prefix))
                    .map(|spec| Pair {
                        display: format!("{} {}", spec.token, spec.argument_shape),
                        replacement: spec.token.to_string(),
                    })
                    .collect(),
            ));
        }
        let start = token_start(prefix);
        let token = &prefix[start..];
        let path_token = token.strip_prefix('@').unwrap_or(token);
        if path_token.is_empty()
            && !prefix.starts_with("/file ")
            && !prefix.starts_with("/image ")
            && !prefix.starts_with("/diff ")
        {
            return Ok((position, Vec::new()));
        }
        let candidates = complete_path(path_token)?
            .into_iter()
            .map(|path| Pair {
                display: path.clone(),
                replacement: if token.starts_with('@') {
                    format!("@{path}")
                } else {
                    path
                },
            })
            .collect();
        Ok((start, candidates))
    }
}

impl Hinter for ChatEditorHelper {
    type Hint = String;
}

impl Highlighter for ChatEditorHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Borrowed(hint)
    }
}

impl Validator for ChatEditorHelper {
    fn validate(&self, context: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        let input = context.input();
        if has_unescaped_trailing_backslash(input) || has_open_code_fence(input) {
            Ok(ValidationResult::Incomplete)
        } else {
            Ok(ValidationResult::Valid(None))
        }
    }
}

pub(crate) fn normalize_multiline_input(input: &str) -> String {
    input.replace("\\\n", "")
}

fn token_start(input: &str) -> usize {
    input
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0)
}

fn complete_path(raw: &str) -> Result<Vec<String>, ReadlineError> {
    let path = Path::new(raw);
    let (directory, name_prefix) = if raw.ends_with(std::path::MAIN_SEPARATOR) {
        (path, "")
    } else {
        (
            path.parent().unwrap_or_else(|| Path::new(".")),
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
        )
    };
    let mut candidates = fs::read_dir(directory)
        .map_err(ReadlineError::Io)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            if !name.starts_with(name_prefix) || sensitive_completion_name(&name) {
                return None;
            }
            let mut candidate = if directory == Path::new(".") {
                PathBuf::from(name)
            } else {
                directory.join(name)
            }
            .to_string_lossy()
            .into_owned();
            if entry.file_type().ok()?.is_dir() {
                candidate.push(std::path::MAIN_SEPARATOR);
            }
            Some(candidate)
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.truncate(100);
    Ok(candidates)
}

fn sensitive_completion_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || [".key", ".pem", ".p12", ".pfx", ".keystore"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn has_unescaped_trailing_backslash(input: &str) -> bool {
    input
        .trim_end_matches([' ', '\t'])
        .chars()
        .rev()
        .take_while(|ch| *ch == '\\')
        .count()
        % 2
        == 1
}

fn has_open_code_fence(input: &str) -> bool {
    input.match_indices("```").count() % 2 == 1
}

#[cfg(test)]
#[path = "chat_editor_tests.rs"]
mod tests;
