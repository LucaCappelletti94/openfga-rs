use super::{ErrorType, Rewrite, Validator, restriction_relation, restrictions};
use alloc::{collections::BTreeMap, format, vec::Vec};
use openfga_model::{TypeDefinition, Userset};

#[derive(Clone, Copy, Default)]
struct Entry {
    reachable: bool,
    cycle: bool,
}

impl Validator<'_, '_> {
    /// Reports relations without a reachable direct assignment.
    pub(super) fn validate_entries(&mut self) {
        let mut visited = Vec::new();
        for definition in &self.model.type_definitions {
            for (relation, rewrite) in &definition.relations {
                let entry = entrypoint(&self.types, definition, relation, rewrite, &mut visited);
                if !entry.reachable {
                    let reason = if entry.cycle {
                        "potential loop"
                    } else {
                        "no entrypoint"
                    };
                    self.relation_error(
                        ErrorType::RelationNoEntrypoint,
                        relation,
                        format!(
                            "`{relation}` is an impossible relation for `{}` ({reason}).",
                            definition.r#type
                        ),
                        definition,
                        relation,
                    );
                }
            }
        }
    }
}

/// Traverses a relation while retaining only the current dependency path.
fn entrypoint<'model>(
    types: &BTreeMap<&'model str, &'model TypeDefinition>,
    definition: &'model TypeDefinition,
    relation: &'model str,
    rewrite: &'model Userset,
    visited: &mut Vec<(&'model str, &'model str)>,
) -> Entry {
    if relation.is_empty() {
        return Entry::default();
    }
    let depth = visited.len();
    visited.push((&definition.r#type, relation));
    let result = inspect(types, definition, relation, rewrite, visited);
    visited.truncate(depth);
    result
}

/// Evaluates the entry-point requirements of a rewrite.
fn inspect<'model>(
    types: &BTreeMap<&'model str, &'model TypeDefinition>,
    definition: &'model TypeDefinition,
    relation: &'model str,
    rewrite: &'model Userset,
    visited: &mut Vec<(&'model str, &'model str)>,
) -> Entry {
    let type_name = definition.r#type.as_str();
    match rewrite.userset.as_ref() {
        Some(Rewrite::This(_)) => direct_entry(types, definition, relation, visited),
        Some(Rewrite::ComputedUserset(computed)) => {
            let target_relation = computed.relation.as_str();
            if target_relation.is_empty() {
                return Entry::default();
            }
            let Some(target) = definition.relations.get(target_relation) else {
                return Entry::default();
            };
            if visited.contains(&(type_name, target_relation)) {
                Entry {
                    reachable: false,
                    cycle: true,
                }
            } else {
                entrypoint(types, definition, target_relation, target, visited)
            }
        }
        Some(Rewrite::TupleToUserset(tuple)) => {
            let Some((parent, target)) =
                tuple.tupleset.as_ref().zip(tuple.computed_userset.as_ref())
            else {
                return Entry::default();
            };
            if parent.relation.is_empty()
                || target.relation.is_empty()
                || !definition.relations.contains_key(&parent.relation)
            {
                return Entry::default();
            }
            for restriction in restrictions(definition, &parent.relation) {
                let target_type = restriction.r#type.as_str();
                let target_relation = target.relation.as_str();
                if let Some(target_definition) = types.get(target_type).copied()
                    && let Some(rewrite) = target_definition.relations.get(target_relation)
                    && !visited.contains(&(target_type, target_relation))
                    && entrypoint(types, target_definition, target_relation, rewrite, visited)
                        .reachable
                {
                    return Entry {
                        reachable: true,
                        cycle: false,
                    };
                }
            }
            Entry::default()
        }
        Some(Rewrite::Union(children)) => {
            let mut cycle = false;
            for child in &children.child {
                let result = entrypoint(types, definition, relation, child, visited);
                if result.reachable {
                    return Entry {
                        reachable: true,
                        cycle: false,
                    };
                }
                cycle |= result.cycle;
            }
            Entry {
                reachable: false,
                cycle,
            }
        }
        Some(Rewrite::Intersection(children)) => {
            for child in &children.child {
                let result = entrypoint(types, definition, relation, child, visited);
                if !result.reachable {
                    return result;
                }
            }
            Entry {
                reachable: true,
                cycle: false,
            }
        }
        Some(Rewrite::Difference(difference)) => {
            let Some(base) = difference.base.as_deref() else {
                return Entry::default();
            };
            let base = entrypoint(types, definition, relation, base, visited);
            if !base.reachable {
                return base;
            }
            let Some(subtract) = difference.subtract.as_deref() else {
                return Entry::default();
            };
            entrypoint(types, definition, relation, subtract, visited)
        }
        _ => Entry::default(),
    }
}

/// Follows userset restrictions until a concrete user type is reachable.
fn direct_entry<'model>(
    types: &BTreeMap<&'model str, &'model TypeDefinition>,
    definition: &'model TypeDefinition,
    relation: &str,
    visited: &mut Vec<(&'model str, &'model str)>,
) -> Entry {
    for restriction in restrictions(definition, relation) {
        let Some(target_relation) = restriction_relation(restriction) else {
            return Entry {
                reachable: true,
                cycle: false,
            };
        };
        let target_type = restriction.r#type.as_str();
        let Some(target_definition) = types.get(target_type).copied() else {
            return Entry::default();
        };
        let Some(target) = target_definition.relations.get(target_relation) else {
            return Entry::default();
        };
        if !visited.contains(&(target_type, target_relation))
            && entrypoint(types, target_definition, target_relation, target, visited).reachable
        {
            return Entry {
                reachable: true,
                cycle: false,
            };
        }
    }
    Entry::default()
}
