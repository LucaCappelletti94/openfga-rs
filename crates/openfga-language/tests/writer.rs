//! Model-to-DSL conformance with the upstream transformer.
use openfga_model::AuthorizationModel;
use std::{fs, path::Path};

/// Modular fixtures retain their ordering and optional source comments.
#[test]
fn modular_transformer_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/openfga-language/tests/data/transformer-module");
    let mut count = 0;
    for entry in fs::read_dir(root).expect("the module corpus is available") {
        let directory = entry.unwrap().path();
        let model_path = directory.join("authorization-model.json");
        let combined_path = directory.join("combined.fga");
        if !directory.is_dir() || !model_path.is_file() || !combined_path.is_file() {
            continue;
        }
        let model: AuthorizationModel =
            serde_json::from_slice(&fs::read(model_path).unwrap()).unwrap();
        assert_eq!(
            openfga_language::write(&model).unwrap().as_bytes(),
            fs::read(combined_path).unwrap(),
            "{}",
            directory.display()
        );
        let source_path = directory.join("combined-sourceinfo.fga");
        if source_path.is_file() {
            assert_eq!(
                openfga_language::write_with_source_information(&model)
                    .unwrap()
                    .as_bytes(),
                fs::read(source_path).unwrap(),
                "{}",
                directory.display()
            );
        }
        count += 1;
    }
    assert_ne!(count, 0, "the module corpus holds no cases");
}

/// The complex module fixture matches both Go writer modes.
#[test]
fn complex_module_go_writer_output() {
    let model: AuthorizationModel = serde_json::from_str(include_str!(
        "../../../conformance/openfga-language/tests/data/transformer-module/06-complex-module/authorization-model.json"
    ))
    .unwrap();
    let expected = r"model
  schema 1.2

type page
  relations
    define owner: [user]
    define space: [space]

type group
  relations
    define member: [user]

type organization
  relations
    define admin: [user]
    define member: [user]
    define can_create_page: member
    define can_create_project: member

type space
  relations
    define can_view_pages: member from organization
    define organization: [organization]

type user

type project
  relations
    define organization: [organization]
    define viewer: member from organization

type ticket
  relations
    define owner: [user]
    define project: [project]

condition a_check(val: int) {
  val <= 10
}
";
    let expected_source = r"model
  schema 1.2

type page
  relations
    define owner: [user]
    define space: [space]

type group # module: bitbucket, file: bitbucket.fga
  relations
    define member: [user]

type organization # module: bitbucket, file: bitbucket.fga
  relations
    define admin: [user]
    define member: [user]
    define can_create_page: member # extended by: module: atlassian, file: atlassian.fga
    define can_create_project: member # extended by: module: jira, file: jira/projects.fga

type space # module: confluence, file: confluence.fga
  relations
    define can_view_pages: member from organization
    define organization: [organization]

type user # module: core, file: core.fga

type project # module: jira, file: jira/projects.fga
  relations
    define organization: [organization]
    define viewer: member from organization

type ticket # module: jira, file: jira/tickets.fga
  relations
    define owner: [user]
    define project: [project]

condition a_check(val: int) {
  val <= 10
} # module: org, file: org.fga
";
    assert_eq!(openfga_language::write(&model).unwrap(), expected);
    assert_eq!(
        openfga_language::write_with_source_information(&model).unwrap(),
        expected_source
    );
}

/// Single-file fixtures retain their canonical output.
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

/// Invalid rewrites retain the upstream diagnostics.
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

/// Collection parameters require an element type.
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

/// Condition names must match their map keys.
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

/// Unknown parameter types retain their numeric representation.
#[test]
fn unknown_parameter_types_are_written_as_numbers() {
    let mut model = AuthorizationModel {
        schema_version: "1.1".into(),
        ..Default::default()
    };
    model.conditions.insert(
        "active".into(),
        openfga_model::Condition {
            name: "active".into(),
            expression: "x".into(),
            parameters: [
                (
                    "x".into(),
                    openfga_model::ConditionParamTypeRef {
                        type_name: 99,
                        generic_types: Vec::new(),
                    },
                ),
                (
                    "y".into(),
                    openfga_model::ConditionParamTypeRef {
                        type_name: openfga_model::condition_param_type_ref::TypeName::List.into(),
                        generic_types: vec![openfga_model::ConditionParamTypeRef {
                            type_name: 42,
                            generic_types: Vec::new(),
                        }],
                    },
                ),
            ]
            .into(),
            ..Default::default()
        },
    );
    assert_eq!(
        openfga_language::write(&model).unwrap(),
        "model\n  schema 1.1\n\ncondition active(x: 99, y: list<42>) {\n  x\n}\n",
    );
}
