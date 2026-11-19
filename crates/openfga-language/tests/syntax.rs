//! Upstream syntax conformance.
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    dsl: String,
    #[serde(default)]
    expected_errors: Vec<serde_norway::Value>,
}

#[test]
fn syntax_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/openfga-language/tests/data/dsl-syntax-validation-cases.yaml");
    let source = std::fs::read_to_string(path)
        .expect("Run git submodule update --init to obtain the conformance corpus");
    let cases: Vec<Case> = serde_norway::from_str(&source).unwrap();
    for case in cases {
        let result = openfga_language::parse(&case.dsl);
        assert_eq!(
            result.is_err(),
            !case.expected_errors.is_empty(),
            "{} {result:?}",
            case.name
        );
    }
}
