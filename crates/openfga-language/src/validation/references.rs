use super::{
    ErrorType, RelationOrWildcard, Rewrite, Validator, restriction_relation, restrictions,
};
use alloc::format;
use openfga_model::{TupleToUserset, TypeDefinition, Userset};

impl Validator<'_, '_> {
    /// Checks restrictions before rewrites in type and relation-name order.
    pub(super) fn validate_references(&mut self) {
        for definition in &self.model.type_definitions {
            if !core::ptr::eq(self.types[definition.r#type.as_str()], definition) {
                continue;
            }
            if let Some(metadata) = &definition.metadata {
                for relation in metadata.relations.keys() {
                    self.validate_direct(definition, relation);
                }
            }
            for (relation, rewrite) in &definition.relations {
                self.validate_rewrite(definition, relation, rewrite);
            }
        }
    }

    /// Checks each leaf of a relation rewrite in depth-first order.
    fn validate_rewrite(&mut self, definition: &TypeDefinition, relation: &str, rewrite: &Userset) {
        match rewrite.userset.as_ref() {
            Some(Rewrite::Union(children) | Rewrite::Intersection(children)) => {
                for child in &children.child {
                    self.validate_rewrite(definition, relation, child);
                }
            }
            Some(Rewrite::Difference(difference)) => {
                for child in [difference.base.as_deref(), difference.subtract.as_deref()]
                    .into_iter()
                    .flatten()
                {
                    self.validate_rewrite(definition, relation, child);
                }
            }
            Some(Rewrite::ComputedUserset(computed)) if !computed.relation.is_empty() => {
                if !self.relations[definition.r#type.as_str()].contains_key(&computed.relation) {
                    self.relation_error(
                        ErrorType::MissingDefinition,
                        &computed.relation,
                        format!("the relation `{}` does not exist.", computed.relation),
                        definition,
                        relation,
                    );
                }
            }
            Some(Rewrite::TupleToUserset(tuple)) => {
                self.validate_tuple(definition, relation, tuple);
            }
            _ => {}
        }
    }

    /// Checks referenced types and their relation restrictions.
    fn validate_direct(&mut self, definition: &TypeDefinition, relation: &str) {
        for restriction in restrictions(definition, relation) {
            if restriction.r#type.is_empty() {
                continue;
            }
            if !self.types.contains_key(restriction.r#type.as_str()) {
                self.relation_error(
                    ErrorType::InvalidType,
                    &restriction.r#type,
                    format!("`{}` is not a valid type.", restriction.r#type),
                    definition,
                    relation,
                );
                continue;
            }
            if let Some(target_relation) = restriction_relation(restriction)
                && !self
                    .relations
                    .get(restriction.r#type.as_str())
                    .is_some_and(|relations| relations.contains_key(target_relation))
            {
                let symbol = format!("{}#{target_relation}", restriction.r#type);
                let line = self
                    .source
                    .as_ref()
                    .and_then(|source| source.relation_line(&definition.r#type, relation));
                self.push(
                    ErrorType::InvalidRelationType,
                    &symbol,
                    format!(
                        "`{target_relation}` is not a valid relation for `{}`.",
                        restriction.r#type
                    ),
                    Some(&restriction.r#type),
                    Some(relation),
                    line,
                );
            }
        }
    }

    /// Checks that a tupleset is direct and exposes the computed relation.
    fn validate_tuple(
        &mut self,
        definition: &TypeDefinition,
        relation: &str,
        tuple: &TupleToUserset,
    ) {
        let target = tuple
            .computed_userset
            .as_ref()
            .map_or("", |computed| computed.relation.as_str());
        let parent = tuple
            .tupleset
            .as_ref()
            .map_or("", |tupleset| tupleset.relation.as_str());
        if parent.is_empty() || target.is_empty() {
            return;
        }
        let symbol = format!("{target} from {parent}");
        let Some(parent_rewrite) = self.relations[definition.r#type.as_str()].get(parent) else {
            self.relation_error(
                ErrorType::InvalidRelationType,
                &symbol,
                format!(
                    "`{parent}` is not a valid relation for `{}`.",
                    definition.r#type
                ),
                definition,
                relation,
            );
            return;
        };
        let allowed = restrictions(definition, parent);
        if !matches!(parent_rewrite.userset, Some(Rewrite::This(_))) || allowed.is_empty() {
            self.indirect_tuple(definition, relation, parent);
            return;
        }
        let mut invalid_targets = 0;
        for restriction in allowed {
            if restriction_relation(restriction).is_some()
                || matches!(
                    restriction.relation_or_wildcard,
                    Some(RelationOrWildcard::Wildcard(_))
                )
            {
                self.indirect_tuple(definition, relation, parent);
            } else if !self
                .relations
                .get(restriction.r#type.as_str())
                .is_some_and(|relations| relations.contains_key(target))
            {
                invalid_targets += 1;
            }
        }
        if invalid_targets == allowed.len() {
            for restriction in allowed {
                self.relation_error(ErrorType::InvalidRelationOnTupleset, &symbol, format!("the `{target}` relation definition on type `{}` is not valid: `{target}` does not exist on `{parent}`, which is of type `{}`.", definition.r#type, restriction.r#type), definition, relation);
            }
        }
    }

    /// Reports a tupleset that cannot contain only direct objects.
    fn indirect_tuple(&mut self, definition: &TypeDefinition, relation: &str, parent: &str) {
        self.relation_error(
            ErrorType::TuplesetNotDirect,
            parent,
            format!("`{parent}` relation used inside from allows only direct relation."),
            definition,
            relation,
        );
    }
}
