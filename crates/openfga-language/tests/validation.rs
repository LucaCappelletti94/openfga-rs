//! Semantic validation conformance.
use openfga_language::{DslError, ValidationErrors, parse, validate, validate_dsl};
use openfga_model::AuthorizationModel;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Case {
    name: String,
    #[serde(default)]
    dsl: String,
    #[serde(default)]
    json: String,
    #[serde(default)]
    skip: bool,
    expected_errors: Option<Vec<Value>>,
}

/// Checks diagnostic rendering and every metadata field supplied by the corpus.
fn check_errors(case: &Case, result: Result<(), ValidationErrors>, positions: bool) {
    let expected = case.expected_errors.as_deref().unwrap_or_default();
    assert_eq!(
        result.is_err(),
        !expected.is_empty(),
        "{} {result:?}",
        case.name
    );
    let Err(errors) = result else { return };
    let descriptions: Vec<String> = expected
        .iter()
        .map(|error| {
            let kind = error["metadata"]["errorType"]
                .as_str()
                .unwrap_or("validation");
            let message = error["msg"].as_str().unwrap();
            if positions {
                format!(
                    "{kind} error at line={}, column={}: {message}",
                    error["line"]["start"], error["column"]["start"]
                )
            } else {
                format!("{kind} error: {message}")
            }
        })
        .collect();
    assert_eq!(
        errors.to_string(),
        format!(
            "{} error{} occurred:\n\t* {}\n\n",
            expected.len(),
            if expected.len() == 1 { "" } else { "s" },
            descriptions.join("\n\t* ")
        ),
        "{}",
        case.name
    );
    assert_eq!(errors.errors.len(), expected.len(), "{}", case.name);
    for (actual, expected) in errors.errors.iter().zip(expected) {
        assert_eq!(
            actual.message,
            expected["msg"].as_str().unwrap(),
            "{}",
            case.name
        );
        let metadata = json!({
            "errorType": actual.error_type.as_str(),
            "symbol": actual.symbol,
            "typeName": actual.type_name,
            "relation": actual.relation,
            "conditionName": actual.condition_name,
        });
        if let Some(expected_metadata) = expected["metadata"].as_object() {
            for (key, value) in expected_metadata {
                assert_eq!(&metadata[key], value, "{} metadata {key}", case.name);
            }
        }
        if positions {
            let position = actual.position.as_ref().expect("DSL diagnostic position");
            assert_eq!(
                json!({"start": position.line.start, "end": position.line.end}),
                expected["line"],
                "{}",
                case.name
            );
            assert_eq!(
                json!({"start": position.column.start, "end": position.column.end}),
                expected["column"],
                "{}",
                case.name
            );
        }
    }
}

/// Checks both validation entry points against semantic DSL cases.
#[test]
fn dsl_semantic_corpus() {
    let cases: Vec<Case> = serde_norway::from_str(include_str!(
        "../../../conformance/openfga-language/tests/data/dsl-semantic-validation-cases.yaml"
    ))
    .unwrap();
    assert!(!cases.is_empty());
    for case in cases.iter().filter(|case| !case.skip) {
        let result = match validate_dsl(&case.dsl) {
            Ok(_) => Ok(()),
            Err(DslError::Semantic(errors)) => Err(errors),
            Err(DslError::Syntax(error)) => panic!("{} syntax error {error}", case.name),
        };
        check_errors(case, result, true);
        let model = parse(&case.dsl).unwrap_or_else(|error| panic!("{} {error}", case.name));
        check_errors(case, validate(&model), false);
    }
}

/// Checks validation of deserialized authorization models.
#[test]
fn json_semantic_corpus() {
    let cases: Vec<Case> = serde_norway::from_str(include_str!(
        "../../../conformance/openfga-language/tests/data/json-validation-cases.yaml"
    ))
    .unwrap();
    assert!(!cases.is_empty());
    for case in cases.iter().filter(|case| !case.skip) {
        let model: AuthorizationModel = serde_json::from_str(&case.json).unwrap_or_else(|error| {
            panic!(
                "{} cannot deserialize into AuthorizationModel {error}",
                case.name
            )
        });
        check_errors(case, validate(&model), false);
    }
}

/// Checks the reference validator result for metadata without a matching relation.
#[test]
fn orphan_relation_metadata() {
    let model: AuthorizationModel = serde_json::from_value(json!({
        "schema_version": "1.1",
        "type_definitions": [
            {"type": "user"},
            {
                "type": "document",
                "relations": {},
                "metadata": {
                    "relations": {
                        "orphan": {
                            "directly_related_user_types": [{"type": "user"}]
                        }
                    }
                }
            }
        ]
    }))
    .unwrap();
    assert_eq!(validate(&model), Ok(()));
}

/// Checks the complete DSL diagnostic text and source spans.
fn assert_dsl_diagnostics(
    source: &str,
    message: &str,
    positions: &[Option<(usize, usize, usize)>],
) {
    let Err(DslError::Semantic(errors)) = validate_dsl(source) else {
        panic!("expected semantic diagnostics");
    };
    assert_eq!(errors.to_string(), message);
    let actual: Vec<_> = errors
        .errors
        .iter()
        .map(|error| {
            error.position.map(|position| {
                assert_eq!(position.line.start, position.line.end);
                (
                    position.line.start,
                    position.column.start,
                    position.column.end,
                )
            })
        })
        .collect();
    assert_eq!(actual, positions);
}

/// Checks the complete model diagnostic text and absence of source positions.
fn assert_json_diagnostics(model: Value, message: &str) {
    let model: AuthorizationModel = serde_json::from_value(model).unwrap();
    let errors = validate(&model).unwrap_err();
    assert_eq!(errors.to_string(), message);
    assert!(errors.errors.iter().all(|error| error.position.is_none()));
}

/// Checks duplicate restrictions that include a named condition.
#[test]
fn duplicate_conditional_restriction() {
    assert_dsl_diagnostics(
        "model\n  schema 1.1\ntype user\ntype document\n  relations\n    define viewer: [user with allowed, user with allowed]\ncondition allowed(value: int) {\n  value > 0\n}\n",
        "1 error occurred:\n\t* duplicated-error error at line=5, column=20: the type restriction `user with allowed` is a duplicate in the relation `viewer`.\n\n",
        &[Some((5, 20, 37))],
    );
}

/// Checks a duplicate whose restriction type occurs after the relation declaration.
#[test]
fn duplicate_before_restriction_type_declaration() {
    assert_dsl_diagnostics(
        "model\n  schema 1.1\ntype document\n  relations\n    define viewer: [user, user]\ntype user\n",
        "1 error occurred:\n\t* duplicated-error error at line=4, column=20: the type restriction `user` is a duplicate in the relation `viewer`.\n\n",
        &[Some((4, 20, 24))],
    );
}

/// Checks that reachable tuplesets and intersections do not hide a cyclic union.
#[test]
fn entrypoints_across_rewrite_operators() {
    assert_dsl_diagnostics(
        "model\n  schema 1.1\ntype user\ntype folder\n  relations\n    define member: [user]\ntype document\n  relations\n    define parent: [folder]\n    define reader: member from parent\n    define writer: [user]\n    define combined: reader and writer\n    define cycle: cycle or cycle2\n    define cycle2: cycle\n",
        "2 errors occurred:\n\t* relation-no-entry-point error at line=12, column=11: `cycle` is an impossible relation for `document` (potential loop).\n\t* relation-no-entry-point error at line=13, column=11: `cycle2` is an impossible relation for `document` (potential loop).\n\n",
        &[Some((12, 11, 16)), Some((13, 11, 17))],
    );
}

/// Checks direct assignments without allowed types.
#[test]
fn direct_relation_without_restrictions() {
    assert_json_diagnostics(
        json!({"schema_version": "1.1", "type_definitions": [{
            "type": "document", "relations": {"viewer": {"this": {}}}
        }]}),
        "1 error occurred:\n\t* relation-no-entry-point error: `viewer` is an impossible relation for `document` (no entrypoint).\n\n",
    );
}

/// Checks partially populated tuple-to-userset references.
#[test]
fn incomplete_tuple_references() {
    for rewrite in [
        json!({"tupleset": {}, "computedUserset": {}}),
        json!({"tupleset": {}, "computedUserset": {"relation": "missing"}}),
        json!({"tupleset": {"relation": "parent"}, "computedUserset": {}}),
    ] {
        assert_json_diagnostics(
            json!({"schema_version": "1.1", "type_definitions": [{
                "type": "document", "relations": {"viewer": {"tupleToUserset": rewrite}}
            }]}),
            "1 error occurred:\n\t* relation-no-entry-point error: `viewer` is an impossible relation for `document` (no entrypoint).\n\n",
        );
    }
}

/// Checks that metadata alone cannot supply a rewrite entry point.
#[test]
fn empty_rewrites_with_restrictions() {
    for rewrite in [json!({}), json!({"computedUserset": {}})] {
        assert_json_diagnostics(
            json!({"schema_version": "1.1", "type_definitions": [
                {"type": "user"},
                {
                    "type": "document",
                    "relations": {"viewer": rewrite},
                    "metadata": {"relations": {"viewer": {
                        "directly_related_user_types": [{"type": "user"}]
                    }}}
                }
            ]}),
            "1 error occurred:\n\t* relation-no-entry-point error: `viewer` is an impossible relation for `document` (no entrypoint).\n\n",
        );
    }
}

/// Checks that one source file can contain several types in the same module.
#[test]
fn shared_module_source() {
    let model: AuthorizationModel = serde_json::from_value(json!({
        "schema_version": "1.1",
        "type_definitions": [
            {"type": "user", "metadata": {
                "module": "shared", "source_info": {"file": "shared.fga"}
            }},
            {"type": "document", "metadata": {
                "module": "shared", "source_info": {"file": "shared.fga"}
            }}
        ]
    }))
    .unwrap();
    assert_eq!(validate(&model), Ok(()));
}

/// Checks a tuple-to-userset rewrite without its tupleset component.
#[test]
fn missing_tupleset_component() {
    assert_json_diagnostics(
        json!({"schema_version": "1.1", "type_definitions": [
            {"type": "user"},
            {
                "type": "document",
                "relations": {"viewer": {"tupleToUserset": {"computedUserset": {}}}},
                "metadata": {"relations": {"viewer": {
                    "directly_related_user_types": [{"type": "user"}]
                }}}
            }
        ]}),
        "1 error occurred:\n\t* relation-no-entry-point error: `viewer` is an impossible relation for `document` (no entrypoint).\n\n",
    );
}

/// Checks a difference rewrite without its subtract operand.
#[test]
fn missing_difference_operand() {
    assert_json_diagnostics(
        json!({"schema_version": "1.1", "type_definitions": [
            {"type": "user"},
            {
                "type": "document",
                "relations": {"viewer": {"difference": {"base": {"this": {}}}}},
                "metadata": {"relations": {"viewer": {
                    "directly_related_user_types": [{"type": "user"}]
                }}}
            }
        ]}),
        "1 error occurred:\n\t* relation-no-entry-point error: `viewer` is an impossible relation for `document` (no entrypoint).\n\n",
    );
}

#[test]
fn retired_schema() {
    assert_json_diagnostics(
        json!({"schema_version": "1.0"}),
        "1 error occurred:\n\t* schema-version-unsupported error: schema version no longer supported\n\n",
    );
}

#[test]
fn empty_type_and_restriction_names() {
    let empty_types = serde_json::from_value(json!({
        "schema_version": "1.1",
        "type_definitions": [{"type": ""}, {"type": ""}]
    }))
    .unwrap();
    assert_eq!(validate(&empty_types), Ok(()));
    assert_json_diagnostics(
        json!({"schema_version": "1.1", "type_definitions": [{
            "type": "", "relations": {"": {"this": {}}}
        }]}),
        "1 error occurred:\n\t* relation-no-entry-point error: `` is an impossible relation for `` (no entrypoint).\n\n",
    );
    for empty_type in [false, true] {
        let mut definitions = vec![json!({
            "type": "document",
            "relations": {"viewer": {"this": {}}},
            "metadata": {"relations": {"viewer": {
                "directly_related_user_types": [
                    {"type": "", "relation": "missing"},
                    {"type": "", "relation": "missing"}
                ]
            }}}
        })];
        if empty_type {
            definitions.push(json!({"type": ""}));
        }
        assert_json_diagnostics(
            json!({"schema_version": "1.1", "type_definitions": definitions}),
            "1 error occurred:\n\t* relation-no-entry-point error: `viewer` is an impossible relation for `document` (no entrypoint).\n\n",
        );
    }
    let empty_restrictions = serde_json::from_value(json!({
        "schema_version": "1.1", "type_definitions": [{
            "type": "document",
            "relations": {"viewer": {"this": {}}},
            "metadata": {"relations": {"viewer": {
                "directly_related_user_types": [{"type": ""}, {"type": ""}]
            }}}
        }]
    }))
    .unwrap();
    assert_eq!(validate(&empty_restrictions), Ok(()));
}

#[test]
fn condition_name_rules() {
    for name in ["*bad".to_owned(), "a".repeat(51)] {
        assert_json_diagnostics(
            json!({"schema_version": "1.1", "conditions": {&name: {"name": name}}}),
            &format!(
                "2 errors occurred:\n\t* invalid-name error: condition '{name}' does not match naming rule: '^[^\\*\\s]{{1,50}}$'.\n\t* condition-not-used error: `{name}` condition is not used in the model.\n\n"
            ),
        );
    }
    for name in [
        "é".repeat(50),
        "\u{b}".to_owned(),
        "\u{a0}".to_owned(),
        "@:#".to_owned(),
    ] {
        assert_json_diagnostics(
            json!({"schema_version": "1.1", "conditions": {&name: {"name": name}}}),
            &format!(
                "1 error occurred:\n\t* condition-not-used error: `{name}` condition is not used in the model.\n\n"
            ),
        );
    }
}

#[test]
fn normalized_module_paths() {
    for (first, second) in [
        ("./folder/../shared.fga", "shared.fga"),
        ("/../../shared.fga", "/shared.fga"),
        ("a/..", "."),
        ("../a/../shared.fga", "../shared.fga"),
    ] {
        assert_json_diagnostics(
            json!({"schema_version": "1.1", "type_definitions": [
                {"type": "first", "metadata": {"module": "first", "source_info": {"file": first}}},
                {"type": "second", "metadata": {"module": "second", "source_info": {"file": second}}}
            ]}),
            &format!(
                "1 error occurred:\n\t* multiple-modules-in-file error: file {second} would contain multiple module definitions (first, second) when transforming to DSL. Only one module can be defined per file.\n\n"
            ),
        );
    }
}

#[test]
fn incomplete_difference_entrypoints() {
    for (difference, reason) in [
        (json!({"subtract": {"this": {}}}), "no entrypoint"),
        (
            json!({"base": {"computedUserset": {"relation": "viewer"}}}),
            "potential loop",
        ),
    ] {
        assert_json_diagnostics(
            json!({"schema_version": "1.1", "type_definitions": [{
                "type": "document", "relations": {"viewer": {"difference": difference}}
            }]}),
            &format!(
                "1 error occurred:\n\t* relation-no-entry-point error: `viewer` is an impossible relation for `document` ({reason}).\n\n"
            ),
        );
    }
}

#[test]
fn diagnostics_follow_relation_name_order() {
    assert_dsl_diagnostics(
        "model\n  schema 1.1\ntype document\n  relations\n    define zebra: missing_zebra\n    define alpha: missing_alpha\n",
        "2 errors occurred:\n\t* missing-definition error at line=5, column=18: the relation `missing_alpha` does not exist.\n\t* missing-definition error at line=4, column=18: the relation `missing_zebra` does not exist.\n\n",
        &[Some((5, 18, 31)), Some((4, 18, 31))],
    );
}

#[test]
fn condition_name_length_dsl() {
    let name = "a".repeat(51);
    assert_dsl_diagnostics(
        &format!(
            "model\n  schema 1.1\ntype user\ncondition {name}(value: int) {{\n value > 0\n}}\n"
        ),
        &format!(
            "2 errors occurred:\n\t* invalid-name error at line=3, column=10: condition '{name}' does not match naming rule: '^[^\\*\\s]{{1,50}}$'.\n\t* condition-not-used error at line=3, column=10: `{name}` condition is not used in the model.\n\n"
        ),
        &[Some((3, 10, 61)), Some((3, 10, 61))],
    );
}

#[test]
fn extended_reserved_type_without_declaration_position() {
    assert_dsl_diagnostics(
        "module core\nextend type self\n  relations\n    define viewer: [self]\n",
        "2 errors occurred:\n\t* schema-version-required error at line=0, column=0: schema version required\n\t* reserved-type-keywords error: a type cannot be named 'self' or 'this'.\n\n",
        &[Some((0, 0, 0)), None],
    );
}

#[test]
fn empty_tuple_operand_in_union() {
    assert_json_diagnostics(
        json!({"schema_version": "1.1", "type_definitions": [
            {"type": "user"},
            {
                "type": "document",
                "relations": {"viewer": {"union": {"child": [
                    {"tupleToUserset": {"computedUserset": {}}}
                ]}}},
                "metadata": {"relations": {"viewer": {
                    "directly_related_user_types": [{"type": "user"}]
                }}}
            }
        ]}),
        "1 error occurred:\n\t* relation-no-entry-point error: `viewer` is an impossible relation for `document` (no entrypoint).\n\n",
    );
}

#[test]
fn shadowed_empty_type_with_missing_computed_relation() {
    assert_json_diagnostics(
        json!({"schema_version": "1.1", "type_definitions": [
            {"type": "", "relations": {"viewer": {"computedUserset": {"relation": "missing"}}}},
            {"type": ""}
        ]}),
        "1 error occurred:\n\t* relation-no-entry-point error: `viewer` is an impossible relation for `` (no entrypoint).\n\n",
    );
}
