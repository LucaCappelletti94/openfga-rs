#![no_std]
#![doc = include_str!("../README.md")]

extern crate alloc;

mod lowering;

use alloc::string::{String, ToString};
use openfga_model::AuthorizationModel;
use pest::{Parser, error::LineColLocation};

mod grammar {
    #[derive(pest_derive::Parser)]
    #[grammar = "openfga.pest"]
    pub(super) struct DslParser;
}

use grammar::{DslParser, Rule};

/// A DSL syntax or transformation error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}, column {column}: {message}")]
pub struct ParseError {
    /// One-based source line.
    pub line: usize,
    /// One-based source column.
    pub column: usize,
    /// Diagnostic description.
    pub message: String,
}

/// Parses an `OpenFGA` DSL model or module into protobuf model types.
///
/// # Errors
/// Returns an error for invalid syntax or duplicate relation, condition, parameter, or extension declarations.
pub fn parse(dsl: &str) -> Result<AuthorizationModel, ParseError> {
    let mut cleaned = String::with_capacity(dsl.len());
    for (index, line) in dsl.split('\n').enumerate() {
        if index != 0 {
            cleaned.push('\n');
        }
        if !line.trim_start_matches(' ').starts_with('#') {
            cleaned.push_str(
                line.split(" #")
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches(' '),
            );
        }
    }
    let source = cleaned.trim_end_matches('\n');
    let pairs = DslParser::parse(Rule::main, source).map_err(|error| {
        let (line, column) = match error.line_col {
            LineColLocation::Pos(position) | LineColLocation::Span(position, _) => position,
        };
        ParseError {
            line,
            column,
            message: error.variant.message().to_string(),
        }
    })?;
    lowering::lower(pairs)
}
