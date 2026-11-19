//! Upstream transformer conformance.
use openfga_model::AuthorizationModel;
use std::{fs, path::Path};

#[test]
fn transformer_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/openfga-language/tests/data/transformer");
    let cases = fs::read_dir(root)
        .expect("Run git submodule update --init to obtain the conformance corpus");
    for entry in cases {
        let directory = entry.unwrap().path();
        if !directory.is_dir() || directory.join("test.skip").exists() {
            continue;
        }
        let source = fs::read_to_string(directory.join("authorization-model.fga")).unwrap();
        let expected: AuthorizationModel = serde_json::from_str(
            &fs::read_to_string(directory.join("authorization-model.json")).unwrap(),
        )
        .unwrap();
        let actual = openfga_language::parse(&source)
            .unwrap_or_else(|error| panic!("{} {error}", directory.display()));
        assert_eq!(actual, expected, "{}", directory.display());
    }
}

#[test]
fn module_metadata_and_inline_conditions() {
    let source = "module access\n\
        type user\n\
        extend type document\n\
          relations\n\
            define order: [user with $expression]\n\
        condition active(value: bool) { value }";
    let expected: AuthorizationModel = serde_json::from_str(
        r#"{
        "type_definitions": [
            {"type": "user", "metadata": {"module": "access"}},
            {"type": "document", "relations": {"order": {"this": {}}},
             "metadata": {"module": "access", "relations": {"order": {
                "module": "access",
                "directly_related_user_types": [{"type": "user", "condition": "$expression"}]
             }}}}
        ],
        "conditions": {"active": {"name": "active", "expression": "value ",
            "parameters": {"value": {"type_name": "TYPE_NAME_BOOL"}},
            "metadata": {"module": "access"}
        }}
    }"#,
    )
    .unwrap();
    assert_eq!(openfga_language::parse(source).unwrap(), expected);
}

#[test]
fn condition_expression_preserves_visible_tokens() {
    let source = "model\n  schema 1.1\n\
        condition active(value: string) {\n  value == \"//{}\" // hidden\n  && value != \"\"\n}";
    let model = openfga_language::parse(source).unwrap();
    assert_eq!(
        model.conditions["active"].expression,
        "value == \"//{}\" \n  && value != \"\""
    );
}

#[test]
fn condition_comments_preserve_surrounding_whitespace() {
    let source = "model\n  schema 1.1\n\
        condition active(value: bool) { // initial\n  // standalone\n  value // trailing\n}\n";
    let expected: AuthorizationModel = serde_json::from_str(
        r#"{
            "schema_version": "1.1",
            "conditions": {"active": {
                "name": "active",
                "expression": "\n  \n  value ",
                "parameters": {"value": {"type_name": "TYPE_NAME_BOOL"}}
            }}
        }"#,
    )
    .unwrap();
    assert_eq!(openfga_language::parse(source).unwrap(), expected);
}
