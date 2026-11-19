use super::{ErrorType, PositionRange, ValidationError, ValidationPosition};
use alloc::{borrow::Cow, format, string::String, vec::Vec};

pub(super) struct Source<'source> {
    lines: Vec<&'source str>,
}

impl<'source> Source<'source> {
    /// Retains source lines for diagnostic lookup.
    pub(super) fn new(source: &'source str) -> Self {
        Self {
            lines: source.split('\n').collect(),
        }
    }

    /// Finds a type declaration by its complete name.
    pub(super) fn type_line(&self, name: &str) -> Option<usize> {
        let prefix = format!("type {name}");
        self.lines
            .iter()
            .position(|line| declaration_matches(line, &prefix))
    }

    /// Finds a relation declaration after its enclosing type.
    pub(super) fn relation_line(&self, type_name: &str, relation: &str) -> Option<usize> {
        let start = self.type_line(type_name).unwrap_or_default();
        let prefix = format!("define {relation}");
        self.lines
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(index, line)| declaration_matches(line, &prefix).then_some(index))
    }

    /// Finds a condition declaration by its complete name.
    pub(super) fn condition_line(&self, name: &str) -> Option<usize> {
        let prefix = format!("condition {name}");
        self.lines
            .iter()
            .position(|line| declaration_matches(line, &prefix))
    }

    /// Finds a schema declaration with an optional trailing comment.
    pub(super) fn schema_line(&self, schema: &str) -> Option<usize> {
        let prefix = format!("schema {schema}");
        self.lines.iter().position(|line| {
            normalize_spaces(line)
                .strip_prefix(&prefix)
                .is_some_and(trailing_comment)
        })
    }

    /// Attaches a zero-based source span to a diagnostic.
    pub(super) fn locate(&self, error: &mut ValidationError, line: Option<usize>) {
        let Some((line, raw_line)) =
            line.and_then(|index| self.lines.get(index).map(|text| (index, *text)))
        else {
            return;
        };
        let symbol = error.symbol.as_str();
        let offset = match error.error_type {
            ErrorType::InvalidType => raw_line.find(':').map_or(0, |separator| {
                separator + 1 + word_offset(&raw_line[separator + 1..], symbol)
            }),
            ErrorType::TuplesetNotDirect => raw_line.find("from").map_or(0, |offset| {
                let start = offset + 4;
                start + raw_line[start..].find(symbol).unwrap_or_default()
            }),
            _ => word_offset(raw_line, symbol),
        };
        error.position = Some(ValidationPosition {
            line: PositionRange {
                start: line,
                end: line,
            },
            column: PositionRange {
                start: offset,
                end: offset + symbol.len(),
            },
        });
    }
}

/// Recognizes an empty suffix or a whitespace-separated comment.
fn trailing_comment(suffix: &str) -> bool {
    suffix.is_empty()
        || (suffix.starts_with(char::is_whitespace) && suffix.trim_start().starts_with('#'))
}

/// Matches a declaration prefix without accepting a longer identifier.
fn declaration_matches(line: &str, prefix: &str) -> bool {
    normalize_spaces(line)
        .strip_prefix(prefix)
        .is_some_and(|suffix| {
            suffix.as_bytes().first().is_none_or(|byte| {
                !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-' | b'/' | b'.')
            })
        })
}

/// Collapses inline lexer whitespace without copying normalized lines.
fn normalize_spaces(line: &str) -> Cow<'_, str> {
    let line = line.trim();
    if !line.contains(['\t', '\u{c}']) && !line.contains("  ") {
        return Cow::Borrowed(line);
    }
    let mut normalized = String::with_capacity(line.len());
    let mut previous_space = false;
    for character in line.chars() {
        let space = matches!(character, ' ' | '\t' | '\u{c}');
        if !space {
            normalized.push(character);
        } else if !previous_space {
            normalized.push(' ');
        }
        previous_space = space;
    }
    Cow::Owned(normalized)
}

/// Prefers ASCII word boundaries, then the first literal occurrence.
fn word_offset(line: &str, symbol: &str) -> usize {
    if symbol.is_empty() {
        return 0;
    }
    if symbol
        .as_bytes()
        .first()
        .copied()
        .is_some_and(word_character)
        && symbol
            .as_bytes()
            .last()
            .copied()
            .is_some_and(word_character)
    {
        for (offset, _) in line.match_indices(symbol) {
            let before = offset
                .checked_sub(1)
                .and_then(|index| line.as_bytes().get(index));
            let after = line.as_bytes().get(offset + symbol.len());
            if !before.copied().is_some_and(word_character)
                && !after.copied().is_some_and(word_character)
            {
                return offset;
            }
        }
    }
    line.find(symbol).unwrap_or_default()
}

/// Recognizes ASCII word characters used by source lookup.
fn word_character(character: u8) -> bool {
    character.is_ascii_alphanumeric() || character == b'_'
}
