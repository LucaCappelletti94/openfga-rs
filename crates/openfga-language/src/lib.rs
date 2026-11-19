#![no_std]
#![doc = include_str!("../README.md")]

extern crate alloc;

mod lowering;
mod mod_file;
mod modules;
mod validation;
mod writer;

pub use mod_file::{
    ModFile, ModFileArrayProperty, ModFileError, ModFileErrors, ModFileStringProperty,
    parse_mod_file,
};
pub use modules::{ModuleError, ModuleErrorKind, ModuleErrors, ModuleFile, merge_modules};
pub use validation::{
    DslError, ErrorType, PositionRange, ValidationError, ValidationErrors, ValidationPosition,
    validate, validate_dsl,
};
pub use writer::{WriteError, write, write_with_source_information};

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
    let cleaned = clean_source(dsl);
    lowering::lower(parse_pairs(&cleaned)?).map(|(model, _)| model)
}

/// Removes DSL comments while retaining source coordinates.
fn clean_source(dsl: &str) -> String {
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
    cleaned
}

/// Parses source whose comments have been removed.
fn parse_pairs(source: &str) -> Result<pest::iterators::Pairs<'_, Rule>, ParseError> {
    DslParser::parse(Rule::main, source.trim_end_matches('\n')).map_err(|error| {
        let (line, column) = match error.line_col {
            LineColLocation::Pos(position) | LineColLocation::Span(position, _) => position,
        };
        ParseError {
            line,
            column,
            message: error.variant.message().to_string(),
        }
    })
}
