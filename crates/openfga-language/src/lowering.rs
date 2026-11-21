use crate::{ParseError, Rule};
use alloc::{
    boxed::Box,
    collections::BTreeSet,
    format,
    string::{String, ToString},
    vec::Vec,
};
use openfga_model::{
    AuthorizationModel, Condition, ConditionMetadata, ConditionParamTypeRef, Difference,
    DirectUserset, Metadata, ObjectRelation, RelationMetadata, RelationReference, TupleToUserset,
    TypeDefinition, Userset, Usersets, Wildcard, condition_param_type_ref::TypeName,
    relation_reference::RelationOrWildcard, userset::Userset as Rewrite,
};
use pest::iterators::{Pair, Pairs};

/// Lowers model declarations and records extended type names.
pub(super) fn lower(
    pairs: Pairs<'_, Rule>,
) -> Result<(AuthorizationModel, BTreeSet<String>), ParseError> {
    let mut model = AuthorizationModel::default();
    let mut module = String::new();
    let mut extensions = BTreeSet::new();
    for pair in pairs.flat_map(Pair::into_inner) {
        match pair.as_rule() {
            Rule::model_header => {
                model.schema_version = tokens(pair).next().unwrap().as_str().to_string();
            }
            Rule::module_header => {
                module = pair.into_inner().next().unwrap().as_str().to_string();
            }
            Rule::type_def => {
                model
                    .type_definitions
                    .push(lower_type(pair, &module, &mut extensions)?);
            }
            Rule::condition => {
                let position = tokens(pair.clone())
                    .next()
                    .unwrap()
                    .into_inner()
                    .next()
                    .unwrap();
                let condition = lower_condition(pair, &module)?;
                if model.conditions.contains_key(&condition.name) {
                    return Err(error(
                        &position,
                        format!(
                            "condition '{}' is already defined in the model",
                            condition.name
                        ),
                    ));
                }
                model.conditions.insert(condition.name.clone(), condition);
            }
            _ => {}
        }
    }
    Ok((model, extensions))
}

fn tokens(pair: Pair<'_, Rule>) -> impl Iterator<Item = Pair<'_, Rule>> {
    pair.into_inner()
        .filter(|token| token.as_rule() != Rule::newline)
}

fn error(pair: &Pair<'_, Rule>, message: String) -> ParseError {
    let (line, column) = pair.as_span().start_pos().line_col();
    ParseError {
        line,
        column,
        message,
    }
}

fn lower_type(
    pair: Pair<'_, Rule>,
    module: &str,
    extensions: &mut BTreeSet<String>,
) -> Result<TypeDefinition, ParseError> {
    let mut children = tokens(pair);
    let first = children.next().unwrap();
    let extension = first.as_rule() == Rule::extend;
    let name = if extension {
        children.next().unwrap()
    } else {
        first
    };
    if extension && module.is_empty() {
        return Err(error(
            &name,
            "extend can only be used in a modular model".to_string(),
        ));
    }
    if extension && !extensions.insert(name.as_str().to_string()) {
        return Err(error(
            &name,
            format!("'{}' is already extended in file.", name.as_str()),
        ));
    }
    let mut definition = TypeDefinition {
        r#type: name.as_str().to_string(),
        ..TypeDefinition::default()
    };
    let mut metadata = Metadata {
        module: module.to_string(),
        ..Metadata::default()
    };
    for declaration in children {
        let mut parts = tokens(declaration);
        let relation = parts.next().unwrap();
        let mut restrictions = Vec::new();
        let rewrite = lower_rewrite(parts.next().unwrap(), &mut restrictions);
        if definition.relations.contains_key(relation.as_str()) {
            return Err(error(
                &relation,
                format!(
                    "'{}' is already defined in '{}'",
                    relation.as_str(),
                    definition.r#type
                ),
            ));
        }
        definition
            .relations
            .insert(relation.as_str().to_string(), rewrite);
        metadata.relations.insert(
            relation.as_str().to_string(),
            RelationMetadata {
                directly_related_user_types: restrictions,
                module: if extension {
                    module.to_string()
                } else {
                    String::new()
                },
                ..RelationMetadata::default()
            },
        );
    }
    if !module.is_empty() || !metadata.relations.is_empty() {
        definition.metadata = Some(metadata);
    }
    Ok(definition)
}

fn lower_rewrite(pair: Pair<'_, Rule>, restrictions: &mut Vec<RelationReference>) -> Userset {
    let rewrite = match pair.as_rule() {
        Rule::direct_assignment => {
            restrictions.clear();
            restrictions.extend(pair.into_inner().map(lower_restriction));
            Rewrite::This(DirectUserset {})
        }
        Rule::rewrite => {
            let mut names = pair.into_inner();
            let computed = ObjectRelation {
                relation: names.next().unwrap().as_str().to_string(),
                ..ObjectRelation::default()
            };
            if let Some(tupleset) = names.next() {
                Rewrite::TupleToUserset(TupleToUserset {
                    tupleset: Some(ObjectRelation {
                        relation: tupleset.as_str().to_string(),
                        ..ObjectRelation::default()
                    }),
                    computed_userset: Some(computed),
                })
            } else {
                Rewrite::ComputedUserset(computed)
            }
        }
        Rule::recurse | Rule::recurse_no_direct => {
            return lower_rewrite(pair.into_inner().next().unwrap(), restrictions);
        }
        rule => {
            assert!(matches!(
                rule,
                Rule::relation_def | Rule::relation_def_no_direct
            ));
            let mut parts = pair.into_inner();
            let first = lower_rewrite(parts.next().unwrap(), restrictions);
            let Some(partials) = parts.next() else {
                return first;
            };
            let mut parts = partials.into_inner();
            let operator = parts.next().unwrap().as_rule();
            let mut children = Vec::new();
            children.push(first);
            for part in parts {
                if !matches!(part.as_rule(), Rule::and | Rule::or | Rule::but_not) {
                    children.push(lower_rewrite(part, restrictions));
                }
            }
            match operator {
                Rule::or => Rewrite::Union(Usersets { child: children }),
                Rule::and => Rewrite::Intersection(Usersets { child: children }),
                operator => {
                    assert_eq!(operator, Rule::but_not);
                    let mut children = children.into_iter();
                    Rewrite::Difference(Box::new(Difference {
                        base: children.next().map(Box::new),
                        subtract: children.next().map(Box::new),
                    }))
                }
            }
        }
    };
    Userset {
        userset: Some(rewrite),
    }
}

fn lower_restriction(pair: Pair<'_, Rule>) -> RelationReference {
    let mut parts = tokens(pair);
    let mut base = parts.next().unwrap().into_inner();
    let mut reference = RelationReference {
        r#type: base.next().unwrap().as_str().to_string(),
        ..RelationReference::default()
    };
    if let Some(suffix) = base.next() {
        reference.relation_or_wildcard = Some(if suffix.as_rule() == Rule::wildcard {
            RelationOrWildcard::Wildcard(Wildcard {})
        } else {
            RelationOrWildcard::Relation(suffix.into_inner().next().unwrap().as_str().to_string())
        });
    }
    if let Some(condition) = parts.next() {
        reference.condition = condition.as_str().to_string();
    }
    reference
}

fn lower_condition(pair: Pair<'_, Rule>, module: &str) -> Result<Condition, ParseError> {
    let mut parts = tokens(pair);
    let mut signature = parts.next().unwrap().into_inner();
    let mut condition = Condition {
        name: signature.next().unwrap().as_str().to_string(),
        ..Condition::default()
    };
    if !module.is_empty() {
        condition.metadata = Some(ConditionMetadata {
            module: module.to_string(),
            ..ConditionMetadata::default()
        });
    }
    for part in signature {
        let mut parameter = part.into_inner();
        let name = parameter.next().unwrap();
        if condition.parameters.contains_key(name.as_str()) {
            return Err(error(
                &name,
                format!(
                    "parameter '{}' is already defined in the condition '{}'",
                    name.as_str(),
                    condition.name
                ),
            ));
        }
        let mut types = parameter.next().unwrap().into_inner();
        let outer = types.next().unwrap();
        condition.parameters.insert(
            name.as_str().to_string(),
            ConditionParamTypeRef {
                type_name: type_number(outer.as_str()),
                generic_types: types
                    .map(|generic| ConditionParamTypeRef {
                        type_name: type_number(generic.as_str()),
                        generic_types: Vec::new(),
                    })
                    .collect(),
            },
        );
    }
    let expression = parts.next().unwrap();
    for token in expression.into_inner() {
        condition.expression.push_str(token.as_str());
    }
    condition
        .expression
        .truncate(condition.expression.trim_end_matches('\n').len());
    Ok(condition)
}

/// Parameter types the grammar accepts, in `TypeName` order.
const PARAMETER_TYPES: [TypeName; 10] = [
    TypeName::Bool,
    TypeName::String,
    TypeName::Int,
    TypeName::Uint,
    TypeName::Double,
    TypeName::Duration,
    TypeName::Timestamp,
    TypeName::Map,
    TypeName::List,
    TypeName::Ipaddress,
];

/// Maps a grammar parameter type keyword to its `TypeName` number.
fn type_number(name: &str) -> i32 {
    PARAMETER_TYPES
        .into_iter()
        .find(|type_name| type_name.as_str_name()["TYPE_NAME_".len()..].eq_ignore_ascii_case(name))
        .expect("the grammar accepts only known parameter types")
        .into()
}
