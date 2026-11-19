//! Every authorization model JSON in the pinned openfga/language corpus reads into the model
//! types and survives both the JSON and the protobuf binary encodings.

use std::fs;
use std::path::{Path, PathBuf};

use openfga_model::AuthorizationModel;
use prost::Message;
use serde_json::Value;

/// JSON names of the map fields in `authzmodel.proto`, whose empty object is the default value.
const MAP_FIELDS: [&str; 3] = ["conditions", "parameters", "relations"];

/// Returns every `authorization-model.json` under the corpus suites holding whole models.
fn corpus_models() -> Vec<PathBuf> {
    let data =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/openfga-language/tests/data");
    let mut models = Vec::new();
    for suite in ["transformer", "transformer-module"] {
        let suite_dir = data.join(suite);
        let cases = fs::read_dir(&suite_dir).unwrap_or_else(|error| {
            panic!(
                "reading {}, run `git submodule update --init`: {error}",
                suite_dir.display()
            )
        });
        for case in cases {
            let model = case
                .expect("a corpus directory entry")
                .path()
                .join("authorization-model.json");
            if model.is_file() {
                models.push(model);
            }
        }
    }
    models.sort();
    models
}

/// Removes every member holding its field's proto3 default, which the JSON mapping may omit.
fn without_defaults(value: Value) -> Value {
    match value {
        Value::Object(members) => Value::Object(
            members
                .into_iter()
                .filter(|(key, member)| !is_default(key, member))
                .map(|(key, member)| (key, without_defaults(member)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_defaults).collect()),
        scalar => scalar,
    }
}

/// Whether `member`, stored under `key`, is the proto3 default for its field.
fn is_default(key: &str, member: &Value) -> bool {
    match member {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(members) => members.is_empty() && MAP_FIELDS.contains(&key),
    }
}

#[test]
fn corpus_models_round_trip() {
    let models = corpus_models();
    assert!(
        !models.is_empty(),
        "the corpus holds no authorization-model.json"
    );
    for path in models {
        let text = fs::read_to_string(&path).expect("a readable corpus model");
        let input: Value = serde_json::from_str(&text).expect("corpus JSON");
        let model: AuthorizationModel = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{} does not read: {error}", path.display()));

        let written = serde_json::to_value(&model).expect("a model writes as JSON");
        assert_eq!(
            written,
            without_defaults(input),
            "{} writes different JSON",
            path.display()
        );

        let decoded = AuthorizationModel::decode(model.encode_to_vec().as_slice())
            .unwrap_or_else(|error| panic!("{} does not decode: {error}", path.display()));
        assert_eq!(
            decoded,
            model,
            "{} changes through protobuf",
            path.display()
        );
    }
}
