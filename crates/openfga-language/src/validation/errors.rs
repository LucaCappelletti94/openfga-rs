use alloc::{string::String, vec::Vec};
use core::fmt;

/// A semantic validation rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ErrorType {
    /// Missing schema version.
    #[error("schema-version-required")]
    SchemaVersionRequired,
    /// Retired schema version.
    #[error("schema-version-unsupported")]
    SchemaVersionUnsupported,
    /// Reserved type name.
    #[error("reserved-type-keywords")]
    ReservedTypeKeywords,
    /// Reserved relation name.
    #[error("reserved-relation-keywords")]
    ReservedRelationKeywords,
    /// Invalid identifier.
    #[error("invalid-name")]
    InvalidName,
    /// Missing relation definition.
    #[error("missing-definition")]
    MissingDefinition,
    /// Undefined relation on a type.
    #[error("invalid-relation-type")]
    InvalidRelationType,
    /// Undefined relation on tupleset targets.
    #[error("invalid-relation-on-tupleset")]
    InvalidRelationOnTupleset,
    /// Undefined type.
    #[error("invalid-type")]
    InvalidType,
    /// Relation without an entry point.
    #[error("relation-no-entry-point")]
    RelationNoEntrypoint,
    /// Indirect tupleset relation.
    #[error("tupleuserset-not-direct")]
    TuplesetNotDirect,
    /// Duplicate definition.
    #[error("duplicated-error")]
    DuplicatedError,
    /// Invalid schema version.
    #[error("invalid-schema")]
    InvalidSchema,
    /// Undefined condition.
    #[error("condition-not-defined")]
    ConditionNotDefined,
    /// Unused condition.
    #[error("condition-not-used")]
    ConditionNotUsed,
    /// Condition key differs from its nested name.
    #[error("different-nested-condition-name")]
    DifferentNestedConditionName,
    /// Multiple modules assigned to one source file.
    #[error("multiple-modules-in-file")]
    MultipleModulesInFile,
}

impl ErrorType {
    /// Returns the stable diagnostic identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchemaVersionRequired => "schema-version-required",
            Self::SchemaVersionUnsupported => "schema-version-unsupported",
            Self::ReservedTypeKeywords => "reserved-type-keywords",
            Self::ReservedRelationKeywords => "reserved-relation-keywords",
            Self::InvalidName => "invalid-name",
            Self::MissingDefinition => "missing-definition",
            Self::InvalidRelationType => "invalid-relation-type",
            Self::InvalidRelationOnTupleset => "invalid-relation-on-tupleset",
            Self::InvalidType => "invalid-type",
            Self::RelationNoEntrypoint => "relation-no-entry-point",
            Self::TuplesetNotDirect => "tupleuserset-not-direct",
            Self::DuplicatedError => "duplicated-error",
            Self::InvalidSchema => "invalid-schema",
            Self::ConditionNotDefined => "condition-not-defined",
            Self::ConditionNotUsed => "condition-not-used",
            Self::DifferentNestedConditionName => "different-nested-condition-name",
            Self::MultipleModulesInFile => "multiple-modules-in-file",
        }
    }
}

/// A zero-based diagnostic range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionRange {
    /// Inclusive start.
    pub start: usize,
    /// Exclusive column end or inclusive line end.
    pub end: usize,
}

/// A diagnostic location in DSL source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidationPosition {
    /// Zero-based line range.
    pub line: PositionRange,
    /// Zero-based UTF-8 byte column range.
    pub column: PositionRange,
}

/// A semantic validation diagnostic.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{error_type} error{location}: {message}", location = Location(.position.as_ref()))]
pub struct ValidationError {
    /// Diagnostic message.
    pub message: String,
    /// Failed rule.
    pub error_type: ErrorType,
    /// Offending symbol.
    pub symbol: String,
    /// Associated type.
    pub type_name: Option<String>,
    /// Associated relation.
    pub relation: Option<String>,
    /// Associated condition.
    pub condition_name: Option<String>,
    /// Source position when validating DSL.
    pub position: Option<ValidationPosition>,
}

struct Location<'source>(Option<&'source ValidationPosition>);

impl fmt::Display for Location<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(position) = self.0 else {
            return Ok(());
        };
        let line = position.line.start;
        let column = position.column.start;
        write!(formatter, " at line={line}, column={column}")
    }
}

/// Diagnostics in rule order, then type declaration order and relation or condition name order.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{diagnostics}", diagnostics = Diagnostics(.errors))]
pub struct ValidationErrors {
    /// Diagnostics in validation order.
    pub errors: Vec<ValidationError>,
}

struct Diagnostics<'errors>(&'errors [ValidationError]);

impl fmt::Display for Diagnostics<'_> {
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

/// A syntax or semantic DSL error.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DslError {
    /// Invalid DSL syntax.
    #[error(transparent)]
    Syntax(#[from] crate::ParseError),
    /// Invalid model semantics.
    #[error(transparent)]
    Semantic(#[from] ValidationErrors),
}
