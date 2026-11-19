mod errors;
mod graph;
mod references;
mod source;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec::Vec,
};
pub use errors::{
    DslError, ErrorType, PositionRange, ValidationError, ValidationErrors, ValidationPosition,
};
use openfga_model::{
    AuthorizationModel, RelationReference, TypeDefinition, Userset,
    relation_reference::RelationOrWildcard, userset::Userset as Rewrite,
};
use source::Source;

/// Validates an authorization model.
///
/// # Errors
/// Returns ordered semantic diagnostics for an invalid model.
pub fn validate(model: &AuthorizationModel) -> Result<(), ValidationErrors> {
    Validator::new(model, None).validate()
}

/// Parses and validates an authorization model in DSL syntax.
///
/// # Errors
/// Returns syntax errors before checking model semantics.
pub fn validate_dsl(dsl: &str) -> Result<AuthorizationModel, DslError> {
    let model = crate::parse(dsl)?;
    Validator::new(&model, Some(Source::new(dsl))).validate()?;
    Ok(model)
}

struct Validator<'model, 'source> {
    model: &'model AuthorizationModel,
    types: BTreeMap<&'model str, &'model TypeDefinition>,
    relations: BTreeMap<&'model str, &'model BTreeMap<String, Userset>>,
    source: Option<Source<'source>>,
    errors: Vec<ValidationError>,
}

impl<'model, 'source> Validator<'model, 'source> {
    /// Indexes type declarations with the last declaration taking precedence.
    fn new(model: &'model AuthorizationModel, source: Option<Source<'source>>) -> Self {
        Self {
            model,
            types: model
                .type_definitions
                .iter()
                .map(|definition| (definition.r#type.as_str(), definition))
                .collect(),
            relations: model
                .type_definitions
                .iter()
                .filter(|definition| !definition.relations.is_empty())
                .map(|definition| (definition.r#type.as_str(), &definition.relations))
                .collect(),
            source,
            errors: Vec::new(),
        }
    }

    /// Runs validation stages in diagnostic precedence order.
    fn validate(mut self) -> Result<(), ValidationErrors> {
        let schema = self.model.schema_version.as_str();
        if schema.is_empty() {
            self.push(
                ErrorType::SchemaVersionRequired,
                "",
                "schema version required".into(),
                None,
                None,
                Some(0),
            );
        } else if !matches!(schema, "1.1" | "1.2") {
            let line = self
                .source
                .as_ref()
                .and_then(|source| source.schema_line(schema));
            self.push(
                if schema == "1.0" {
                    ErrorType::SchemaVersionUnsupported
                } else {
                    ErrorType::InvalidSchema
                },
                schema,
                if schema == "1.0" {
                    "schema version no longer supported".into()
                } else {
                    format!("invalid schema {schema}")
                },
                None,
                None,
                line,
            );
        }
        self.validate_names();
        self.validate_references();
        if self.errors.is_empty() {
            self.validate_duplicates();
        }
        if self.errors.is_empty() {
            self.validate_entries();
        }
        self.validate_modules();
        self.validate_conditions();
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors {
                errors: self.errors,
            })
        }
    }

    /// Appends a diagnostic with its optional source position.
    fn push(
        &mut self,
        error_type: ErrorType,
        symbol: &str,
        message: String,
        type_name: Option<&str>,
        relation: Option<&str>,
        line: Option<usize>,
    ) {
        let mut error = ValidationError {
            message,
            error_type,
            symbol: symbol.into(),
            type_name: type_name.map(ToString::to_string),
            relation: relation.map(ToString::to_string),
            condition_name: (error_type == ErrorType::ConditionNotUsed).then(|| symbol.into()),
            position: None,
        };
        if let Some(source) = &self.source {
            source.locate(&mut error, line);
        }
        self.errors.push(error);
    }

    /// Appends a diagnostic attached to a relation definition.
    fn relation_error(
        &mut self,
        error_type: ErrorType,
        symbol: &str,
        message: String,
        definition: &TypeDefinition,
        relation: &str,
    ) {
        let line = self
            .source
            .as_ref()
            .and_then(|source| source.relation_line(&definition.r#type, relation));
        self.push(
            error_type,
            symbol,
            message,
            Some(&definition.r#type),
            Some(relation),
            line,
        );
    }

    /// Checks reserved names and identifier bounds.
    fn validate_names(&mut self) {
        for definition in &self.model.type_definitions {
            let name = definition.r#type.as_str();
            if name.is_empty() {
                continue;
            }
            let line = self
                .source
                .as_ref()
                .and_then(|source| source.type_line(name));
            if matches!(name, "self" | "this") {
                self.push(
                    ErrorType::ReservedTypeKeywords,
                    name,
                    "a type cannot be named 'self' or 'this'.".into(),
                    None,
                    None,
                    line,
                );
            }
            if !valid_name(name, 254) {
                self.push(
                    ErrorType::InvalidName,
                    name,
                    format!(r"type '{name}' does not match naming rule: '^[^:#@\*\s]{{1,254}}$'."),
                    None,
                    None,
                    line,
                );
            }
            for relation in definition.relations.keys() {
                let line = self
                    .source
                    .as_ref()
                    .and_then(|source| source.relation_line(name, relation));
                if matches!(relation.as_str(), "self" | "this") {
                    self.push(
                        ErrorType::ReservedRelationKeywords,
                        relation,
                        "a relation cannot be named 'self' or 'this'.".into(),
                        None,
                        None,
                        line,
                    );
                }
                if !valid_name(relation, 50) {
                    self.push(ErrorType::InvalidName, relation, format!(r"relation '{relation}' of type '{name}' does not match naming rule: '^[^:#@\*\s]{{1,50}}$'."), None, None, line);
                }
            }
        }
        for name in self.model.conditions.keys() {
            if !valid_condition_name(name) {
                let line = self
                    .source
                    .as_ref()
                    .and_then(|source| source.condition_line(name));
                self.push(
                    ErrorType::InvalidName,
                    name,
                    format!(r"condition '{name}' does not match naming rule: '^[^\*\s]{{1,50}}$'."),
                    None,
                    None,
                    line,
                );
            }
        }
    }

    /// Checks duplicate types, restrictions, and immediate rewrite children.
    fn validate_duplicates(&mut self) {
        let mut names = BTreeSet::new();
        for definition in &self.model.type_definitions {
            let name = definition.r#type.as_str();
            if name.is_empty() {
                continue;
            }
            if !names.insert(name) {
                let line = self
                    .source
                    .as_ref()
                    .and_then(|source| source.type_line(name));
                self.push(
                    ErrorType::DuplicatedError,
                    name,
                    format!("the type `{name}` is a duplicate."),
                    Some(name),
                    None,
                    line,
                );
            }
            let Some(metadata) = &definition.metadata else {
                continue;
            };
            for (relation, metadata) in &metadata.relations {
                let mut restrictions = BTreeSet::new();
                for restriction in &metadata.directly_related_user_types {
                    if restriction.r#type.is_empty() {
                        continue;
                    }
                    let key = (
                        restriction.r#type.as_str(),
                        restriction_relation(restriction),
                        matches!(
                            restriction.relation_or_wildcard,
                            Some(RelationOrWildcard::Wildcard(_))
                        ),
                        restriction.condition.as_str(),
                    );
                    if !restrictions.insert(key) {
                        let symbol = restriction_name(restriction);
                        let line = self
                            .source
                            .as_ref()
                            .and_then(|source| source.relation_line(name, relation));
                        self.push(ErrorType::DuplicatedError, &symbol, format!("the type restriction `{symbol}` is a duplicate in the relation `{relation}`."), Some(name), Some(relation), line);
                    }
                }
                let Some(rewrite) = definition
                    .relations
                    .get(relation)
                    .and_then(|userset| userset.userset.as_ref())
                else {
                    continue;
                };
                match rewrite {
                    Rewrite::Union(children) | Rewrite::Intersection(children) => {
                        let mut names = BTreeSet::new();
                        for child in &children.child {
                            if let Some(key) = rewrite_key(child)
                                && !names.insert(key)
                            {
                                self.duplicate_relation(&rewrite_symbol(key), definition, relation);
                            }
                        }
                    }
                    Rewrite::Difference(difference) => {
                        if let Some(key) = difference.base.as_deref().and_then(rewrite_key)
                            && difference.subtract.as_deref().and_then(rewrite_key) == Some(key)
                        {
                            self.duplicate_relation(&rewrite_symbol(key), definition, relation);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Reports a repeated immediate rewrite child.
    fn duplicate_relation(&mut self, symbol: &str, definition: &TypeDefinition, relation: &str) {
        self.relation_error(ErrorType::DuplicatedError, symbol, format!("the partial relation definition `{symbol}` is a duplicate in the relation `{relation}`."), definition, relation);
    }

    /// Checks condition names and references from restrictions.
    fn validate_conditions(&mut self) {
        let mut used: BTreeMap<&str, Vec<(&TypeDefinition, &str)>> = BTreeMap::new();
        for definition in &self.model.type_definitions {
            if let Some(metadata) = &definition.metadata {
                for (relation, metadata) in &metadata.relations {
                    for restriction in &metadata.directly_related_user_types {
                        if !restriction.condition.is_empty() {
                            used.entry(&restriction.condition)
                                .or_default()
                                .push((definition, relation));
                        }
                    }
                }
            }
        }
        for (condition, uses) in &used {
            if *condition != "$expression" && !self.model.conditions.contains_key(*condition) {
                for (definition, relation) in uses {
                    self.relation_error(
                        ErrorType::ConditionNotDefined,
                        condition,
                        format!("`{condition}` is not a defined condition in the model."),
                        definition,
                        relation,
                    );
                }
            }
        }
        for (name, condition) in &self.model.conditions {
            if name != &condition.name {
                self.push(
                    ErrorType::DifferentNestedConditionName,
                    &condition.name,
                    format!(
                        "condition key is `{name}` but nested name property is {}",
                        condition.name
                    ),
                    None,
                    None,
                    None,
                );
            }
        }
        for name in self.model.conditions.keys() {
            if !used.contains_key(name.as_str()) {
                let line = self
                    .source
                    .as_ref()
                    .and_then(|source| source.condition_line(name));
                self.push(
                    ErrorType::ConditionNotUsed,
                    name,
                    format!("`{name}` condition is not used in the model."),
                    None,
                    None,
                    line,
                );
            }
        }
    }

    /// Checks that source-file metadata names at most one module.
    fn validate_modules(&mut self) {
        let mut modules = Vec::new();
        for definition in &self.model.type_definitions {
            if let Some(metadata) = &definition.metadata {
                record_module(
                    &mut modules,
                    metadata.source_info.as_ref(),
                    &metadata.module,
                );
            }
        }
        for definition in &self.model.type_definitions {
            if let Some(metadata) = &definition.metadata {
                for (name, relation) in &metadata.relations {
                    if definition.relations.contains_key(name) {
                        record_module(
                            &mut modules,
                            relation
                                .source_info
                                .as_ref()
                                .filter(|source| !source.file.is_empty())
                                .or(metadata.source_info.as_ref()),
                            if relation.module.is_empty() {
                                &metadata.module
                            } else {
                                &relation.module
                            },
                        );
                    }
                }
            }
        }
        for condition in self.model.conditions.values() {
            if let Some(metadata) = &condition.metadata {
                record_module(
                    &mut modules,
                    metadata.source_info.as_ref(),
                    &metadata.module,
                );
            }
        }
        for (file, modules) in modules {
            if modules.len() > 1 {
                self.push(ErrorType::MultipleModulesInFile, &file, format!("file {file} would contain multiple module definitions ({}) when transforming to DSL. Only one module can be defined per file.", modules.join(", ")), None, None, None);
            }
        }
    }
}

/// Records distinct modules in discovery order for each source file.
fn record_module<'model>(
    modules: &mut Vec<(String, Vec<&'model str>)>,
    source: Option<&'model openfga_model::SourceInfo>,
    module: &'model str,
) {
    if let Some(source) = source.filter(|source| !source.file.is_empty() && !module.is_empty()) {
        let file = clean_file(&source.file);
        if let Some((_, names)) = modules.iter_mut().find(|(name, _)| *name == file) {
            if !names.contains(&module) {
                names.push(module);
            }
        } else {
            modules.push((file, alloc::vec![module]));
        }
    }
}

/// Normalizes Unix source paths without accessing a filesystem.
fn clean_file(file: &str) -> String {
    let absolute = file.starts_with('/');
    let mut parts = Vec::new();
    for part in file.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|part| *part != "..") => {
                parts.pop();
            }
            ".." if absolute => {}
            _ => parts.push(part),
        }
    }
    let mut normalized = if absolute {
        String::from("/")
    } else {
        String::new()
    };
    normalized.push_str(&parts.join("/"));
    if normalized.is_empty() {
        normalized.push('.');
    }
    normalized
}

/// Checks identifier characters and Unicode scalar length.
fn valid_name(name: &str, maximum: usize) -> bool {
    !name.is_empty()
        && name.chars().count() <= maximum
        && !name.chars().any(|character| {
            matches!(
                character,
                ':' | '#' | '@' | '*' | ' ' | '\t' | '\n' | '\r' | '\u{c}'
            )
        })
}

/// Checks a condition's name characters and length.
fn valid_condition_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 50
        && !name
            .chars()
            .any(|character| matches!(character, '*' | ' ' | '\t' | '\n' | '\r' | '\u{c}'))
}

/// Borrows the direct restrictions of a relation.
fn restrictions<'model>(
    definition: &'model TypeDefinition,
    relation: &str,
) -> &'model [RelationReference] {
    definition
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.relations.get(relation))
        .map_or(&[], |metadata| {
            metadata.directly_related_user_types.as_slice()
        })
}

/// Borrows a nonempty relation restriction.
fn restriction_relation(restriction: &RelationReference) -> Option<&str> {
    match restriction.relation_or_wildcard.as_ref() {
        Some(RelationOrWildcard::Relation(relation)) if !relation.is_empty() => Some(relation),
        _ => None,
    }
}

/// Renders a restriction for a diagnostic.
fn restriction_name(restriction: &RelationReference) -> String {
    let mut name = restriction.r#type.clone();
    match restriction.relation_or_wildcard.as_ref() {
        Some(RelationOrWildcard::Relation(relation)) if !relation.is_empty() => {
            name.push('#');
            name.push_str(relation);
        }
        Some(RelationOrWildcard::Wildcard(_)) => name.push_str(":*"),
        _ => {}
    }
    if !restriction.condition.is_empty() {
        name.push_str(" with ");
        name.push_str(&restriction.condition);
    }
    name
}

/// Identifies a computed or tuple-to-userset rewrite without allocation.
fn rewrite_key(userset: &Userset) -> Option<(&str, Option<&str>)> {
    match userset.userset.as_ref()? {
        Rewrite::ComputedUserset(computed) if !computed.relation.is_empty() => {
            Some((&computed.relation, None))
        }
        Rewrite::TupleToUserset(tuple) => {
            let target = tuple.computed_userset.as_ref()?.relation.as_str();
            if target.is_empty() {
                return None;
            }
            let parent = tuple
                .tupleset
                .as_ref()
                .map(|tupleset| tupleset.relation.as_str())
                .filter(|parent| !parent.is_empty());
            Some((target, parent))
        }
        _ => None,
    }
}

/// Renders a rewrite key for a diagnostic.
fn rewrite_symbol((target, parent): (&str, Option<&str>)) -> String {
    parent.map_or_else(|| target.into(), |parent| format!("{target} from {parent}"))
}
