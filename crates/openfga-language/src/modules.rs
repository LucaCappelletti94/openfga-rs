use crate::{ParseError, PositionRange, clean_source, lowering, parse_pairs};
use alloc::{
    collections::BTreeSet,
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;
use openfga_model::{AuthorizationModel, SourceInfo, TypeDefinition};

/// A named DSL source borrowed for modular transformation.
#[derive(Clone, Copy, Debug)]
pub struct ModuleFile<'source> {
    /// Source file name.
    pub name: &'source str,
    /// DSL module contents.
    pub contents: &'source str,
}

/// The stage producing a module diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModuleErrorKind {
    /// Invalid DSL syntax or declarations.
    #[error("syntax")]
    Syntax,
    /// Incompatible declarations across files.
    #[error("transformation")]
    Transformation,
}

/// A diagnostic located in a modular source file.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{kind} error at line={line_start}, column={column_start}: {message}", line_start = .line.start, column_start = .column.start)]
pub struct ModuleError {
    /// Diagnostic description.
    pub message: String,
    /// Source file name.
    pub file: String,
    /// Zero-based line range.
    pub line: PositionRange,
    /// Zero-based column range.
    pub column: PositionRange,
    /// Diagnostic category.
    pub kind: ModuleErrorKind,
}

/// Module diagnostics in transformation order.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{diagnostics}", diagnostics = Diagnostics(.errors))]
pub struct ModuleErrors {
    /// Individual diagnostics.
    pub errors: Vec<ModuleError>,
}

struct Diagnostics<'errors>(&'errors [ModuleError]);

impl fmt::Display for Diagnostics<'_> {
    /// Formats the complete diagnostic list.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.0.len();
        let plural = if count == 1 { "" } else { "s" };
        write!(formatter, "{count} error{plural} occurred:\n\t* ")?;
        for (index, error) in self.0.iter().enumerate() {
            if index != 0 {
                formatter.write_str("\n\t* ")?;
            }
            write!(formatter, "{error}")?;
        }
        formatter.write_str("\n\n")
    }
}

/// Combines modular declarations without validating model semantics.
///
/// # Errors
/// Returns syntax errors and conflicting or missing modular declarations.
pub fn merge_modules(
    modules: &[ModuleFile<'_>],
    schema_version: &str,
) -> Result<AuthorizationModel, ModuleErrors> {
    let mut model = AuthorizationModel {
        schema_version: schema_version.to_string(),
        ..AuthorizationModel::default()
    };
    let mut errors = Vec::new();
    let mut types = BTreeSet::new();
    let mut extensions = Vec::new();
    for module in modules {
        let cleaned = clean_source(module.contents);
        let parsed = parse_pairs(&cleaned)
            .and_then(|pairs| lowering::lower(pairs.clone()).map(|lowered| (pairs, lowered)));
        let (pairs, (mut parsed, extended)) = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                errors.push(syntax_error(module, error));
                continue;
            }
        };
        if !parsed.schema_version.is_empty() {
            errors.push(ModuleError {
                message: "file is not a module".to_string(),
                file: module.name.to_string(),
                line: PositionRange { start: 0, end: 0 },
                column: PositionRange { start: 0, end: 0 },
                kind: ModuleErrorKind::Transformation,
            });
            continue;
        }
        for mut definition in parsed.type_definitions {
            if extended.contains(&definition.r#type) {
                extensions.push((module, definition));
                continue;
            }
            if !types.insert(definition.r#type.clone()) {
                errors.push(declaration_error(
                    module,
                    "type",
                    &definition.r#type,
                    format!("duplicate type definition {}", definition.r#type),
                ));
                continue;
            }
            definition
                .metadata
                .get_or_insert_with(Default::default)
                .source_info = Some(SourceInfo {
                file: module.name.to_string(),
            });
            model.type_definitions.push(definition);
        }
        for declaration in pairs
            .flat_map(pest::iterators::Pair::into_inner)
            .filter(|pair| pair.as_rule() == crate::Rule::condition)
        {
            let name = condition_name(declaration);
            merge_condition(
                &mut model,
                &mut parsed.conditions,
                module,
                name,
                &mut errors,
            );
        }
    }
    for (module, definition) in extensions {
        if let Some(original) = model
            .type_definitions
            .iter_mut()
            .find(|original| original.r#type == definition.r#type)
        {
            merge_extension(original, definition, module, &mut errors);
        } else {
            errors.push(declaration_error(
                module,
                "extend type",
                &definition.r#type,
                format!("extended type {} does not exist", definition.r#type),
            ));
        }
    }
    if errors.is_empty() {
        Ok(model)
    } else {
        Err(ModuleErrors { errors })
    }
}

/// Adds extension relations while retaining their source metadata.
fn merge_extension(
    original: &mut TypeDefinition,
    mut extension: TypeDefinition,
    module: &ModuleFile<'_>,
    errors: &mut Vec<ModuleError>,
) {
    let mut metadata = extension.metadata.unwrap();
    let declarations = module
        .contents
        .lines()
        .skip_while(|line| declaration_name(line, "extend type") != Some(extension.r#type.as_str()))
        .skip(1)
        .take_while(|line| {
            declaration_name(line, "type").is_none()
                && declaration_name(line, "extend type").is_none()
                && declaration_name(line, "condition").is_none()
        });
    for name in declarations.filter_map(|line| declaration_name(line, "define")) {
        let relation = extension.relations.remove(name).unwrap();
        if original.relations.contains_key(name) {
            errors.push(declaration_error(
                module,
                "define",
                name,
                format!("relation {name} already exists on type {}", original.r#type),
            ));
            continue;
        }
        let mut relation_metadata = metadata.relations.remove(name).unwrap();
        relation_metadata.source_info = Some(SourceInfo {
            file: module.name.to_string(),
        });
        original
            .metadata
            .as_mut()
            .unwrap()
            .relations
            .insert(name.to_string(), relation_metadata);
        original.relations.insert(name.to_string(), relation);
    }
}

/// Extracts an identifier following a declaration keyword sequence.
fn declaration_name<'source>(source: &'source str, keyword: &str) -> Option<&'source str> {
    let mut words = source.split_whitespace();
    if !keyword
        .split(' ')
        .all(|keyword| words.next() == Some(keyword))
    {
        return None;
    }
    words.next()?.split([':', '(']).next()
}

/// Returns the name token of a parsed condition declaration.
fn condition_name(pair: pest::iterators::Pair<'_, crate::Rule>) -> &str {
    pair.into_inner()
        .find(|pair| pair.as_rule() != crate::Rule::newline)
        .unwrap()
        .into_inner()
        .next()
        .unwrap()
        .as_str()
}

/// Transfers one parsed condition and reports duplicate names.
fn merge_condition(
    model: &mut AuthorizationModel,
    conditions: &mut alloc::collections::BTreeMap<String, openfga_model::Condition>,
    module: &ModuleFile<'_>,
    name: &str,
    errors: &mut Vec<ModuleError>,
) {
    if model.conditions.contains_key(name) {
        errors.push(declaration_error(
            module,
            "condition",
            name,
            format!("duplicate condition {name}"),
        ));
        return;
    }
    let mut condition = conditions.remove(name).unwrap();
    condition
        .metadata
        .get_or_insert_with(Default::default)
        .source_info = Some(SourceInfo {
        file: module.name.to_string(),
    });
    model.conditions.insert(name.to_string(), condition);
}

/// Locates a declaration by its complete keyword sequence and identifier.
fn declaration_error(
    module: &ModuleFile<'_>,
    keyword: &str,
    name: &str,
    message: String,
) -> ModuleError {
    let (line, source) = module
        .contents
        .lines()
        .enumerate()
        .find(|(_, source)| {
            let mut words = source.split_whitespace();
            keyword
                .split(' ')
                .all(|keyword| words.next() == Some(keyword))
                && words
                    .next()
                    .is_some_and(|word| word.split([':', '(']).next() == Some(name))
        })
        .unwrap();
    let column = source.find(name).unwrap();
    ModuleError {
        message,
        file: module.name.to_string(),
        line: PositionRange {
            start: line,
            end: line,
        },
        column: PositionRange {
            start: column,
            end: column + name.len(),
        },
        kind: ModuleErrorKind::Transformation,
    }
}

/// Converts parser coordinates into zero-based module diagnostics.
fn syntax_error(module: &ModuleFile<'_>, error: ParseError) -> ModuleError {
    let line = error.line - 1;
    let column = error.column - 1;
    let width = module
        .contents
        .lines()
        .nth(line)
        .unwrap_or_default()
        .chars()
        .skip(column)
        .take_while(|character| {
            character.is_alphanumeric() || matches!(character, '_' | '-' | '.' | '/')
        })
        .count();
    ModuleError {
        message: error.message,
        file: module.name.to_string(),
        line: PositionRange {
            start: line,
            end: line,
        },
        column: PositionRange {
            start: column,
            end: column + width,
        },
        kind: ModuleErrorKind::Syntax,
    }
}
