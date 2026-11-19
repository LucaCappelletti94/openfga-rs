//! Model-to-DSL conformance with the upstream transformer.
use openfga_model::AuthorizationModel;
use std::{fs, path::Path};

#[test]
fn transformer_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/openfga-language/tests/data/transformer");
    let cases = fs::read_dir(root)
        .expect("Run git submodule update --init to obtain the conformance corpus");
    let mut count = 0;
    for entry in cases {
        let directory = entry.unwrap().path();
        if !directory.is_dir() || directory.join("test.skip").exists() {
            continue;
        }
        let expected = fs::read_to_string(directory.join("authorization-model.fga")).unwrap();
        let model: AuthorizationModel = serde_json::from_str(
            &fs::read_to_string(directory.join("authorization-model.json")).unwrap(),
        )
        .unwrap_or_else(|error| panic!("{} {error}", directory.display()));
        let actual = openfga_language::write(&model)
            .unwrap_or_else(|error| panic!("{} {error}", directory.display()));
        assert_eq!(actual, expected, "{}", directory.display());
        let parsed = openfga_language::parse(&actual)
            .unwrap_or_else(|error| panic!("{} {error}", directory.display()));
        assert_eq!(parsed, model, "{}", directory.display());
        count += 1;
    }
    assert_ne!(count, 0, "the transformer corpus holds no cases");
}

#[derive(serde::Deserialize)]
struct SyntaxCase {
    name: String,
    json: String,
    #[serde(default)]
    error_message: String,
}

#[test]
fn json_syntax_corpus() {
    let source = include_str!(
        "../../../conformance/openfga-language/tests/data/json-syntax-transformer-validation-cases.yaml"
    );
    let cases: Vec<SyntaxCase> = serde_norway::from_str(source).unwrap();
    assert!(!cases.is_empty(), "the JSON syntax corpus holds no cases");
    for case in cases {
        let model: AuthorizationModel = serde_json::from_str(&case.json)
            .unwrap_or_else(|error| panic!("{} {error}", case.name));
        let result = openfga_language::write(&model);
        if case.error_message.is_empty() {
            assert!(result.is_ok(), "{} {result:?}", case.name);
        } else {
            let error = result.expect_err(&case.name);
            assert_eq!(error.to_string(), case.error_message, "{}", case.name);
        }
    }
}

#[test]
fn collection_parameter_requires_element_type() {
    let mut model = AuthorizationModel::default();
    model.conditions.insert(
        "active".into(),
        openfga_model::Condition {
            name: "active".into(),
            parameters: [(
                "items".into(),
                openfga_model::ConditionParamTypeRef {
                    type_name: openfga_model::condition_param_type_ref::TypeName::List.into(),
                    generic_types: Vec::new(),
                },
            )]
            .into(),
            ..Default::default()
        },
    );
    assert_eq!(
        openfga_language::write(&model),
        Err(openfga_language::WriteError::MissingGenericType {
            condition_name: "active".into(),
            parameter_name: "items".into(),
        }),
    );
}

#[test]
fn condition_name_must_match_map_key() {
    let mut model = AuthorizationModel::default();
    model.conditions.insert(
        "active".into(),
        openfga_model::Condition {
            name: "other".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        openfga_language::write(&model).unwrap_err().to_string(),
        "the 'active' condition has a different nested condition name ('other')",
    );
}
