use alloc::{string::String, vec::Vec};
use core::fmt::Write as _;
use openfga_model::{
    AuthorizationModel, RelationReference, SourceInfo, Userset, condition_param_type_ref::TypeName,
    relation_reference::RelationOrWildcard, userset::Userset as Rewrite,
};

/// A model that cannot be represented in the `OpenFGA` DSL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// A relation contains an unsupported rewrite or direct assignment position.
    #[error(
        "the '{relation_name}' relation definition under the '{type_name}' type is not supported by the OpenFGA DSL syntax yet"
    )]
    UnsupportedNesting {
        /// The enclosing type.
        type_name: String,
        /// The enclosing relation.
        relation_name: String,
    },
    /// A condition map key differs from its nested name.
    #[error(
        "the '{condition_name}' condition has a different nested condition name ('{nested_name}')"
    )]
    ConditionNameMismatch {
        /// The condition map key.
        condition_name: String,
        /// The nested condition name.
        nested_name: String,
    },
    /// A collection parameter has no element type.
    #[error("the '{parameter_name}' parameter of condition '{condition_name}' has no generic type")]
    MissingGenericType {
        /// The enclosing condition.
        condition_name: String,
        /// The collection parameter.
        parameter_name: String,
    },
}

/// Writes a model as `OpenFGA` DSL without source-information comments.
///
/// # Errors
/// Returns an error for unsupported rewrites, mismatched condition names, or missing collection element types.
pub fn write(model: &AuthorizationModel) -> Result<String, WriteError> {
    write_model(model, false)
}

/// Writes a model as `OpenFGA` DSL with source-information comments.
///
/// # Errors
/// Returns an error for unsupported rewrites, mismatched condition names, or missing collection element types.
pub fn write_with_source_information(model: &AuthorizationModel) -> Result<String, WriteError> {
    write_model(model, true)
}

/// Writes a model using the requested source-comment policy.
fn write_model(
    model: &AuthorizationModel,
    include_source_information: bool,
) -> Result<String, WriteError> {
    let mut output = String::from("model\n  schema ");
    output.push_str(&model.schema_version);
    output.push('\n');
    write_types(&mut output, model, include_source_information)?;
    write_conditions(&mut output, model, include_source_information)?;
    Ok(output)
}

/// Writes type definitions and their relations in model order.
fn write_types(
    output: &mut String,
    model: &AuthorizationModel,
    include_source_information: bool,
) -> Result<(), WriteError> {
    let modular = model.type_definitions.iter().any(|definition| {
        definition
            .metadata
            .as_ref()
            .is_some_and(|metadata| !metadata.module.is_empty())
    });
    let definitions = modular_order(model.type_definitions.iter(), modular, |definition| {
        let (module, source) = definition.metadata.as_ref().map_or(("", None), |metadata| {
            (metadata.module.as_str(), metadata.source_info.as_ref())
        });
        source_order(&definition.r#type, module, source)
    });
    for definition in definitions {
        output.push_str("\ntype ");
        output.push_str(&definition.r#type);
        let (module, source) = definition.metadata.as_ref().map_or(("", None), |metadata| {
            (metadata.module.as_str(), metadata.source_info.as_ref())
        });
        write_source_comment(output, module, source, false, include_source_information);
        if !definition.relations.is_empty() {
            output.push_str("\n  relations");
        }
        let relations = modular_order(definition.relations.iter(), modular, |(name, _)| {
            let (module, source) = definition
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.relations.get(*name))
                .map_or(("", None), |metadata| {
                    (metadata.module.as_str(), metadata.source_info.as_ref())
                });
            source_order(name, module, source)
        });
        for (relation_name, rewrite) in relations {
            let metadata = definition
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.relations.get(relation_name));
            let restrictions = metadata.map_or(&[][..], |metadata| {
                metadata.directly_related_user_types.as_slice()
            });
            output.push_str("\n    define ");
            output.push_str(relation_name);
            output.push_str(": ");
            let mut occurrences = 0;
            let result =
                write_rewrite(output, Some(rewrite), restrictions, &mut occurrences, false);
            if result.is_err() || (occurrences != 0 && (occurrences != 1 || !first_direct(rewrite)))
            {
                return Err(WriteError::UnsupportedNesting {
                    type_name: definition.r#type.clone(),
                    relation_name: relation_name.clone(),
                });
            }
            let (module, source) = metadata.map_or(("", None), |metadata| {
                (metadata.module.as_str(), metadata.source_info.as_ref())
            });
            write_source_comment(output, module, source, true, include_source_information);
        }
        output.push('\n');
    }
    Ok(())
}

/// Writes conditions in module, file, and name order.
fn write_conditions(
    output: &mut String,
    model: &AuthorizationModel,
    include_source_information: bool,
) -> Result<(), WriteError> {
    let modular_conditions = model.conditions.values().any(|condition| {
        condition
            .metadata
            .as_ref()
            .is_some_and(|metadata| !metadata.module.is_empty())
    });
    let conditions = modular_order(
        model.conditions.iter(),
        modular_conditions,
        |(name, condition)| {
            let (module, source) = condition.metadata.as_ref().map_or(("", None), |metadata| {
                (metadata.module.as_str(), metadata.source_info.as_ref())
            });
            source_order(name, module, source)
        },
    );
    for (condition_name, condition) in conditions {
        if condition_name != &condition.name {
            return Err(WriteError::ConditionNameMismatch {
                condition_name: condition_name.clone(),
                nested_name: condition.name.clone(),
            });
        }
        output.push_str("\ncondition ");
        output.push_str(condition_name);
        output.push('(');
        for (index, (parameter_name, parameter)) in condition.parameters.iter().enumerate() {
            if index != 0 {
                output.push_str(", ");
            }
            output.push_str(parameter_name);
            output.push_str(": ");
            write_type_name(output, parameter.type_name);
            if matches!(
                TypeName::try_from(parameter.type_name),
                Ok(TypeName::List | TypeName::Map)
            ) {
                let generic = parameter.generic_types.first().ok_or_else(|| {
                    WriteError::MissingGenericType {
                        condition_name: condition_name.clone(),
                        parameter_name: parameter_name.clone(),
                    }
                })?;
                output.push('<');
                write_type_name(output, generic.type_name);
                output.push('>');
            }
        }
        output.push_str(") {\n  ");
        output.push_str(&condition.expression);
        output.push_str("\n}");
        let (module, source) = condition.metadata.as_ref().map_or(("", None), |metadata| {
            (metadata.module.as_str(), metadata.source_info.as_ref())
        });
        write_source_comment(output, module, source, false, include_source_information);
        output.push('\n');
    }
    Ok(())
}

/// Orders modular entries without collecting single-file entries.
fn modular_order<'a, Entry, Entries, Key>(
    mut entries: Entries,
    modular: bool,
    key: Key,
) -> impl Iterator<Item = Entry>
where
    Entries: ExactSizeIterator<Item = Entry>,
    Key: Fn(&Entry) -> (&'a str, &'a str, &'a str),
{
    let mut sorted = Vec::new();
    if modular && entries.len() > 1 {
        sorted.extend(entries.by_ref());
        sorted.sort_by_key(key);
    }
    entries.chain(sorted)
}

/// Sorts unassigned entries by name and modular entries by module, file, and name.
fn source_order<'a>(
    name: &'a str,
    module: &'a str,
    source: Option<&'a SourceInfo>,
) -> (&'a str, &'a str, &'a str) {
    let file = if module.is_empty() {
        ""
    } else {
        source.map_or("", |source| source.file.as_str())
    };
    (module, file, name)
}

/// Appends available source information when requested.
fn write_source_comment(
    output: &mut String,
    module: &str,
    source: Option<&SourceInfo>,
    relation: bool,
    include_source_information: bool,
) {
    let file = source.map_or("", |source| source.file.as_str());
    if include_source_information && (!module.is_empty() || !file.is_empty()) {
        output.push_str(if relation { " # extended by:" } else { " #" });
        output.push_str(" module: ");
        output.push_str(module);
        output.push_str(", file: ");
        output.push_str(file);
    }
}

/// Writes a parameter type name or its unknown numeric value.
fn write_type_name(output: &mut String, number: i32) {
    if let Ok(name) = TypeName::try_from(number) {
        for character in name.as_str_name().trim_start_matches("TYPE_NAME_").chars() {
            output.push(character.to_ascii_lowercase());
        }
    } else {
        let _ = write!(output, "{number}");
    }
}

/// Reports whether a rewrite is direct assignment.
fn is_direct(userset: &Userset) -> bool {
    matches!(userset.userset, Some(Rewrite::This(_)))
}

/// Reports whether direct assignment occupies the first permitted position.
fn first_direct(userset: &Userset) -> bool {
    match userset.userset.as_ref() {
        Some(Rewrite::This(_)) => true,
        Some(Rewrite::Difference(difference)) => {
            difference.base.as_deref().is_some_and(first_direct)
        }
        Some(Rewrite::Union(children) | Rewrite::Intersection(children)) => {
            children.child.iter().any(is_direct) || children.child.first().is_some_and(first_direct)
        }
        _ => false,
    }
}

/// Writes direct-assignment type restrictions.
fn write_restrictions(output: &mut String, restrictions: &[RelationReference]) {
    output.push('[');
    for (index, restriction) in restrictions.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        output.push_str(&restriction.r#type);
        match restriction.relation_or_wildcard.as_ref() {
            Some(RelationOrWildcard::Wildcard(_)) => output.push_str(":*"),
            Some(RelationOrWildcard::Relation(relation)) if !relation.is_empty() => {
                output.push('#');
                output.push_str(relation);
            }
            _ => {}
        }
        if !restriction.condition.is_empty() {
            output.push_str(" with ");
            output.push_str(&restriction.condition);
        }
    }
    output.push(']');
}

/// Writes a relation rewrite and counts direct assignments.
fn write_rewrite(
    output: &mut String,
    userset: Option<&Userset>,
    restrictions: &[RelationReference],
    occurrences: &mut usize,
    nested: bool,
) -> Result<(), ()> {
    let rewrite = userset
        .and_then(|userset| userset.userset.as_ref())
        .ok_or(())?;
    let parenthesized = nested
        && matches!(
            rewrite,
            Rewrite::Union(_) | Rewrite::Intersection(_) | Rewrite::Difference(_)
        );
    if parenthesized {
        output.push('(');
    }
    match rewrite {
        Rewrite::This(_) => {
            *occurrences += 1;
            write_restrictions(output, restrictions);
        }
        Rewrite::ComputedUserset(computed) => output.push_str(&computed.relation),
        Rewrite::TupleToUserset(tuple) => {
            output.push_str(
                tuple
                    .computed_userset
                    .as_ref()
                    .map_or("", |computed| computed.relation.as_str()),
            );
            output.push_str(" from ");
            output.push_str(
                tuple
                    .tupleset
                    .as_ref()
                    .map_or("", |tupleset| tupleset.relation.as_str()),
            );
        }
        Rewrite::Difference(difference) => {
            write_rewrite(
                output,
                difference.base.as_deref(),
                restrictions,
                occurrences,
                true,
            )?;
            output.push_str(" but not ");
            write_rewrite(
                output,
                difference.subtract.as_deref(),
                restrictions,
                occurrences,
                true,
            )?;
        }
        Rewrite::Union(children) | Rewrite::Intersection(children) => {
            let separator = if matches!(rewrite, Rewrite::Union(_)) {
                " or "
            } else {
                " and "
            };
            let direct_position = children.child.iter().position(is_direct);
            let prioritized = direct_position
                .into_iter()
                .chain((0..children.child.len()).filter(|index| Some(*index) != direct_position));
            for (position, index) in prioritized.enumerate() {
                if position != 0 {
                    output.push_str(separator);
                }
                write_rewrite(
                    output,
                    Some(&children.child[index]),
                    restrictions,
                    occurrences,
                    true,
                )?;
            }
        }
    }
    if parenthesized {
        output.push(')');
    }
    Ok(())
}
