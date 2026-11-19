use alloc::string::String;
use core::fmt::Write as _;
use openfga_model::{
    AuthorizationModel, RelationReference, Userset, condition_param_type_ref::TypeName,
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

/// Writes a single-file model as `OpenFGA` DSL without source-information comments.
///
/// # Errors
/// Returns an error for unsupported rewrites, mismatched condition names, or missing collection element types.
pub fn write(model: &AuthorizationModel) -> Result<String, WriteError> {
    let mut output = String::from("model\n  schema ");
    output.push_str(&model.schema_version);
    output.push('\n');
    for definition in &model.type_definitions {
        output.push_str("\ntype ");
        output.push_str(&definition.r#type);
        if !definition.relations.is_empty() {
            output.push_str("\n  relations");
        }
        for (relation_name, rewrite) in &definition.relations {
            let restrictions = definition
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.relations.get(relation_name))
                .map_or(&[][..], |metadata| {
                    metadata.directly_related_user_types.as_slice()
                });
            output.push_str("\n    define ");
            output.push_str(relation_name);
            output.push_str(": ");
            let mut occurrences = 0;
            let result = write_rewrite(
                &mut output,
                Some(rewrite),
                restrictions,
                &mut occurrences,
                false,
            );
            if result.is_err() || (occurrences != 0 && (occurrences != 1 || !first_direct(rewrite)))
            {
                return Err(WriteError::UnsupportedNesting {
                    type_name: definition.r#type.clone(),
                    relation_name: relation_name.clone(),
                });
            }
        }
        output.push('\n');
    }
    for (condition_name, condition) in &model.conditions {
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
            write_type_name(&mut output, parameter.type_name);
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
                write_type_name(&mut output, generic.type_name);
                output.push('>');
            }
        }
        output.push_str(") {\n  ");
        output.push_str(&condition.expression);
        output.push_str("\n}\n");
    }
    Ok(output)
}

fn write_type_name(output: &mut String, number: i32) {
    if let Ok(name) = TypeName::try_from(number) {
        for character in name.as_str_name().trim_start_matches("TYPE_NAME_").chars() {
            output.push(character.to_ascii_lowercase());
        }
    } else {
        let _ = write!(output, "{number}");
    }
}

fn is_direct(userset: &Userset) -> bool {
    matches!(userset.userset, Some(Rewrite::This(_)))
}

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
