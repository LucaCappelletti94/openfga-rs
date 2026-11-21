//! Module manifest values, diagnostics, and source positions.

use openfga_language::{
    ModFile, ModFileArrayProperty, ModFileError, ModFileErrors, ModFileStringProperty,
    parse_mod_file,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    #[serde(rename = "modFile")]
    source: String,
    json: Option<String>,
    #[serde(default)]
    expected_errors: Vec<ExpectedError>,
}

#[derive(Deserialize)]
struct Start {
    start: usize,
}

#[derive(Deserialize)]
struct ExpectedProperty {
    value: String,
    line: Start,
    column: Start,
}

#[derive(Deserialize)]
struct ExpectedArray {
    value: Vec<ExpectedProperty>,
    line: Start,
    column: Start,
}

#[derive(Deserialize)]
struct ExpectedModel {
    schema: ExpectedProperty,
    contents: ExpectedArray,
}

#[derive(Deserialize)]
struct ExpectedError {
    msg: String,
    line: Start,
    column: Start,
}

/// Converts corpus source ranges to zero-based start positions.
fn property(expected: ExpectedProperty) -> ModFileStringProperty {
    ModFileStringProperty {
        value: expected.value,
        line: expected.line.start,
        column: expected.column.start,
    }
}

/// Every pinned manifest case preserves values and complete diagnostics.
#[test]
fn corpus_cases() {
    let cases: Vec<Case> = serde_norway::from_str(include_str!(
        "../../../conformance/openfga-language/tests/data/fga-mod-transformer-cases.yaml"
    ))
    .expect("manifest corpus");
    assert_eq!(cases.len(), 9);
    for case in cases {
        if let Some(json) = case.json {
            let expected: ExpectedModel = serde_json::from_str(&json).expect("expected model");
            assert_eq!(
                parse_mod_file(&case.source).expect(&case.name),
                ModFile {
                    schema: property(expected.schema),
                    contents: ModFileArrayProperty {
                        value: expected.contents.value.into_iter().map(property).collect(),
                        line: expected.contents.line.start,
                        column: expected.contents.column.start,
                    },
                },
                "{}",
                case.name
            );
        } else {
            let expected: Vec<_> = case
                .expected_errors
                .into_iter()
                .map(|error| ModFileError {
                    message: error.msg,
                    line: error.line.start,
                    column: error.column.start,
                })
                .collect();
            let errors = parse_mod_file(&case.source).expect_err(&case.name);
            let ModFileErrors::Validation { errors: actual } = &errors else {
                panic!("{} returned YAML diagnostics", case.name);
            };
            assert_eq!(*actual, expected, "{}", case.name);
            assert_eq!(
                errors.to_string(),
                display_errors(&expected),
                "{}",
                case.name
            );
        }
    }
}

/// Formats the aggregate diagnostic contract.
fn display_errors(errors: &[ModFileError]) -> String {
    format!(
        "{} error{} occurred:\n\t* {}\n\n",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" },
        errors
            .iter()
            .map(|error| format!(
                "validation error at line={}, column={}: {}",
                error.line, error.column, error.message
            ))
            .collect::<Vec<_>>()
            .join("\n\t* ")
    )
}

/// Checks complete ordered diagnostics and their positions.
fn assert_errors(source: &str, expected: &[(usize, usize, &str)]) {
    let expected: Vec<_> = expected
        .iter()
        .map(|&(line, column, message)| ModFileError {
            message: message.to_owned(),
            line,
            column,
        })
        .collect();
    let actual = parse_mod_file(source).expect_err(source);
    let ModFileErrors::Validation { errors: validation } = &actual else {
        panic!("expected validation diagnostics");
    };
    assert_eq!(*validation, expected);
    assert_eq!(actual.to_string(), display_errors(&expected));
}

/// Empty documents report both absent properties.
#[test]
fn empty_documents() {
    for source in ["", "null"] {
        assert_errors(
            source,
            &[
                (0, 0, "missing schema field"),
                (0, 0, "missing contents field"),
            ],
        );
    }
}

/// URI query decoding replaces plus signs and normalizes separators.
#[test]
fn decoded_paths_and_tag_positions() {
    let actual = parse_mod_file(
        "schema: !!str 1.2\ncontents: [a+b.fga, 'folder\\core%2efga', 'a%2Bb.fga', '%FF.fga']\n",
    )
    .expect("valid paths");
    assert_eq!(
        actual,
        ModFile {
            schema: ModFileStringProperty {
                value: "1.2".into(),
                line: 0,
                column: 8
            },
            contents: ModFileArrayProperty {
                value: [
                    ("a b.fga", 11),
                    ("folder/core.fga", 20),
                    ("a+b.fga", 41),
                    ("�.fga", 54)
                ]
                .into_iter()
                .map(|(value, column)| ModFileStringProperty {
                    value: value.into(),
                    line: 1,
                    column
                })
                .collect(),
                line: 1,
                column: 10,
            },
        }
    );
}

/// Invalid percent escapes retain the original path and its position.
#[test]
fn malformed_percent_escapes() {
    assert_errors(
        "schema: !!str 1.2\ncontents: [a+b.fga, 'folder\\core%2efga', '%ZZ.fga', '%', '%2', '%FF.fga']\n",
        &[
            (1, 41, "failed to decode path: %ZZ.fga"),
            (1, 52, "failed to decode path: %"),
            (1, 57, "failed to decode path: %2"),
        ],
    );
}

/// YAML scalar tags determine field and item types.
#[test]
fn scalar_and_collection_types() {
    assert_errors(
        "schema: 1.2\ncontents: []\n",
        &[(
            0,
            8,
            "unexpected schema type, expected string got value 1.2",
        )],
    );
    assert_errors(
        "schema: null\ncontents: null\n",
        &[
            (
                0,
                8,
                "unexpected schema type, expected string got value null",
            ),
            (
                1,
                10,
                "unexpected contents type, expected list of strings got value null",
            ),
        ],
    );
    assert_errors(
        "schema: {}\ncontents: {}\n",
        &[
            (0, 8, "unexpected schema type, expected string got value "),
            (
                1,
                10,
                "unexpected contents type, expected list of strings got value ",
            ),
        ],
    );
    assert_errors(
        "schema: '1.2'\ncontents: [null, {}, [], 2026-01-01, yes, 0xFF, .inf]\n",
        &[
            (
                1,
                11,
                "unexpected contents item type, expected string got value null",
            ),
            (
                1,
                17,
                "unexpected contents item type, expected string got value ",
            ),
            (
                1,
                21,
                "unexpected contents item type, expected string got value ",
            ),
            (
                1,
                25,
                "unexpected contents item type, expected string got value 2026-01-01",
            ),
            (
                1,
                37,
                "contents items should use fga file extension, got yes",
            ),
            (
                1,
                42,
                "unexpected contents item type, expected string got value 0xFF",
            ),
            (
                1,
                48,
                "unexpected contents item type, expected string got value .inf",
            ),
        ],
    );
    assert_errors(
        "schema: '1.2'\ncontents: !!str [a.fga]\n",
        &[(
            1,
            10,
            "unexpected contents type, expected list of strings got value ",
        )],
    );
    assert_errors(
        "schema: '1.2'\ncontents: [!!str true, !custom a.fga]\n",
        &[
            (
                1,
                11,
                "contents items should use fga file extension, got true",
            ),
            (
                1,
                23,
                "unexpected contents item type, expected string got value a.fga",
            ),
        ],
    );
}

/// Anchors retain their start position while field aliases remain alias nodes.
#[test]
fn anchors_and_aliases() {
    assert_eq!(
        parse_mod_file("schema: &version '1.2'\ncontents: [&path a.fga]\n")
            .expect("anchored values"),
        ModFile {
            schema: ModFileStringProperty {
                value: "1.2".into(),
                line: 0,
                column: 8
            },
            contents: ModFileArrayProperty {
                value: vec![ModFileStringProperty {
                    value: "a.fga".into(),
                    line: 1,
                    column: 11
                }],
                line: 1,
                column: 10
            },
        }
    );
    assert_errors(
        "schema: &version '1.2'\ncontents: [&path a.fga, *path]\n",
        &[(
            1,
            24,
            "unexpected contents item type, expected string got value path",
        )],
    );
    assert_errors(
        "version: &version '1.2'\nschema: *version\ncontents: []\n",
        &[(
            1,
            8,
            "unexpected schema type, expected string got value version",
        )],
    );
}

/// Merge mappings inherit values and source positions.
#[test]
fn merge_mapping() {
    assert_eq!(
        parse_mod_file("defaults: &defaults {schema: '1.2', contents: [a.fga]}\n<<: *defaults\n")
            .expect("merged fields"),
        ModFile {
            schema: ModFileStringProperty {
                value: "1.2".into(),
                line: 0,
                column: 29
            },
            contents: ModFileArrayProperty {
                value: vec![ModFileStringProperty {
                    value: "a.fga".into(),
                    line: 0,
                    column: 47
                }],
                line: 0,
                column: 46
            },
        }
    );
}

/// Only the first YAML document supplies manifest properties.
#[test]
fn first_document_and_empty_contents() {
    let expected = ModFile {
        schema: ModFileStringProperty {
            value: "1.2".into(),
            line: 0,
            column: 8,
        },
        contents: ModFileArrayProperty {
            value: vec![],
            line: 1,
            column: 10,
        },
    };
    for source in [
        "schema: '1.2'\ncontents: []\n",
        "schema: '1.2'\ncontents: []\n---\ninvalid: [\n",
    ] {
        assert_eq!(parse_mod_file(source).expect("first document"), expected);
    }
}

/// Duplicate root keys retain their YAML diagnostic.
#[test]
fn duplicate_keys() {
    let errors = parse_mod_file("schema: '1.2'\nschema: '1.2'\ncontents: []\n")
        .expect_err("duplicate schema");
    assert_eq!(
        errors.to_string(),
        "yaml: unmarshal errors:\n  line 2: mapping key \"schema\" already defined at line 1"
    );
}

/// Traversal checks apply after normalization and inspect substrings.
#[test]
fn embedded_traversal() {
    assert_errors(
        "schema: '1.2'\ncontents: [C:\\core.fga, './a.fga', 'a..fga', '.../a.fga', 'a%2Bb.fga']\n",
        &[(1, 45, "invalid contents item .../a.fga")],
    );
}

/// Scalar resolution distinguishes invalid dates and overflowing numbers.
#[test]
fn scalar_resolution_boundaries() {
    assert_errors(
        "schema: '1.2'\ncontents: [2024-2-29, 2023-2-29, --1, +01, 1e400, 0b101, 0o77]\n",
        &[
            (
                1,
                11,
                "unexpected contents item type, expected string got value 2024-2-29",
            ),
            (
                1,
                22,
                "contents items should use fga file extension, got 2023-2-29",
            ),
            (
                1,
                33,
                "contents items should use fga file extension, got --1",
            ),
            (
                1,
                38,
                "unexpected contents item type, expected string got value +01",
            ),
            (
                1,
                43,
                "contents items should use fga file extension, got 1e400",
            ),
            (
                1,
                50,
                "unexpected contents item type, expected string got value 0b101",
            ),
            (
                1,
                57,
                "unexpected contents item type, expected string got value 0o77",
            ),
        ],
    );
}

/// Block scalar diagnostics point to the scalar indicator.
#[test]
fn block_scalar_indicator_position() {
    assert_errors(
        "schema: |\n  1.2\ncontents: []\n",
        &[(
            0,
            8,
            "unsupported schema version, fga.mod only supported in version `1.2`",
        )],
    );
}

/// Unicode source columns count characters rather than UTF-8 bytes.
#[test]
fn unicode_positions() {
    assert_eq!(
        parse_mod_file("schema: '1.2'\ncontents: [é.fga, '猫.fga', 'x.fga']\n")
            .expect("Unicode paths"),
        ModFile {
            schema: ModFileStringProperty {
                value: "1.2".into(),
                line: 0,
                column: 8
            },
            contents: ModFileArrayProperty {
                value: [("é.fga", 11), ("猫.fga", 18), ("x.fga", 27)]
                    .into_iter()
                    .map(|(value, column)| ModFileStringProperty {
                        value: value.into(),
                        line: 1,
                        column
                    })
                    .collect(),
                line: 1,
                column: 10,
            },
        }
    );
}

/// Invalid root values preserve YAML decoding diagnostics.
#[test]
fn invalid_root_values() {
    for (source, message) in [
        (
            "[]",
            "yaml: unmarshal errors:\n  line 1: cannot unmarshal !!seq into transformer.YAMLModFile",
        ),
        (
            "hello",
            "yaml: unmarshal errors:\n  line 1: cannot unmarshal !!str `hello` into transformer.YAMLModFile",
        ),
    ] {
        let errors = parse_mod_file(source).expect_err(source);
        assert_eq!(errors.to_string(), message);
        let ModFileErrors::Decode { line, column, .. } = errors else {
            panic!("expected decoding error");
        };
        assert_eq!((line, column), (0, 0));
    }
}

/// Duplicate keys accumulate before field validation.
#[test]
fn multiple_duplicate_keys() {
    let errors = parse_mod_file("schema: '1.2'\nschema: '1.2'\ncontents: []\ncontents: []\n")
        .expect_err("duplicate properties");
    assert_eq!(
        errors.to_string(),
        "yaml: unmarshal errors:\n  line 2: mapping key \"schema\" already defined at line 1\n  line 4: mapping key \"contents\" already defined at line 3"
    );
}

/// Merge sequences accept only mappings or aliases to mappings.
#[test]
fn invalid_merge_values() {
    for source in [
        "schema: '1.2'\ncontents: []\n<<: [[]]\n",
        "base: &base 1\nschema: '1.2'\ncontents: []\n<<: *base\n",
    ] {
        assert_eq!(
            parse_mod_file(source).expect_err(source).to_string(),
            "yaml: map merge requires map or sequence of maps as the value"
        );
    }
}

/// Non-specific tags retain implicit node types.
#[test]
fn non_specific_tags() {
    assert_eq!(
        parse_mod_file("schema: ! '1.2'\ncontents: ! [! a.fga]\n").expect("non-specific tags"),
        ModFile {
            schema: ModFileStringProperty {
                value: "1.2".into(),
                line: 0,
                column: 8
            },
            contents: ModFileArrayProperty {
                value: vec![ModFileStringProperty {
                    value: "a.fga".into(),
                    line: 1,
                    column: 13
                }],
                line: 1,
                column: 10
            },
        }
    );
    assert_errors(
        "schema: '1.2'\ncontents: [! 1.2]\n",
        &[(
            1,
            11,
            "unexpected contents item type, expected string got value 1.2",
        )],
    );
}

/// Invalid UTF-8 escapes produce one replacement character per byte.
#[test]
fn invalid_utf8_escapes() {
    assert_eq!(
        parse_mod_file("schema: '1.2'\ncontents: ['%E1%80.fga']\n").expect("escaped bytes"),
        ModFile {
            schema: ModFileStringProperty {
                value: "1.2".into(),
                line: 0,
                column: 8
            },
            contents: ModFileArrayProperty {
                value: vec![ModFileStringProperty {
                    value: "��.fga".into(),
                    line: 1,
                    column: 11
                }],
                line: 1,
                column: 10
            },
        }
    );
}

/// Malformed YAML exposes crate-owned zero-based diagnostics.
#[test]
fn malformed_yaml_position() {
    let errors =
        parse_mod_file("schema: '1.2'\ncontents: [a.fga}\n").expect_err("malformed sequence");
    let ModFileErrors::Syntax(error) = errors else {
        panic!("expected syntax error");
    };
    assert_eq!((error.line, error.column), (1, 16));
}

/// Malformed numbers and timestamps remain string values.
#[test]
fn malformed_scalar_spellings() {
    assert_errors(
        "schema: '1.2'\ncontents: [++1, -+1, 0x+1, 2024-+1-1, 2024-01-001, 2024-1-1T1:1:1e1Z]\n",
        &[
            (
                1,
                11,
                "contents items should use fga file extension, got ++1",
            ),
            (
                1,
                16,
                "contents items should use fga file extension, got -+1",
            ),
            (
                1,
                21,
                "contents items should use fga file extension, got 0x+1",
            ),
            (
                1,
                27,
                "contents items should use fga file extension, got 2024-+1-1",
            ),
            (
                1,
                38,
                "contents items should use fga file extension, got 2024-01-001",
            ),
            (
                1,
                51,
                "contents items should use fga file extension, got 2024-1-1T1:1:1e1Z",
            ),
        ],
    );
}

/// Plain numeric and timestamp spellings retain their resolved types.
#[test]
fn timestamp_and_numeric_resolution() {
    for scalar in [
        "1_000",
        "2024-4-30",
        "2024-1-2T3:4:5.25Z",
        "2024-1-2 3:4:5.25",
        "2024-1-2T3:4:5+02:30",
        "2024-1-2T3:4:5-02:30",
    ] {
        let source = format!("schema: '1.2'\ncontents: [{scalar}]\n");
        let message = format!("unexpected contents item type, expected string got value {scalar}");
        assert_errors(&source, &[(1, 11, &message)]);
    }
    for scalar in [
        "2024-4-31",
        "2024-1-2T3:4:5",
        "2024-1-2 3:4:5Z",
        "2024-1-2T3:4:5.Z",
        "2024-1-2T3:4:5.xZ",
        "2024-1-2T24:4:5Z",
        "2024-1-2T3:60:5Z",
        "2024-1-2T3:4:60Z",
        "2024-1-2T3:4:5:6Z",
        "2024-1-2T3:4:5+02",
        "2024-1-2T3:4:5+02-30",
        "2024-1-2T3:4:5+25:30",
        "2024-1-2T3:4:5+02:61",
    ] {
        let source = format!("schema: '1.2'\ncontents: [{scalar}]\n");
        let message = format!("contents items should use fga file extension, got {scalar}");
        assert_errors(&source, &[(1, 11, &message)]);
    }
}

/// Merge sequences combine fields while preserving explicit declarations.
#[test]
fn merge_sequence_precedence() {
    assert_eq!(
        parse_mod_file("<<: [{schema: '1.2'}, {contents: [a.fga]}]\n").expect("merged sequence"),
        ModFile {
            schema: ModFileStringProperty {
                value: "1.2".into(),
                line: 0,
                column: 14
            },
            contents: ModFileArrayProperty {
                value: vec![ModFileStringProperty {
                    value: "a.fga".into(),
                    line: 0,
                    column: 34
                }],
                line: 0,
                column: 33,
            },
        }
    );
    assert_eq!(
        parse_mod_file("schema: '1.2'\ncontents: [direct.fga]\n<<: [{schema: '1.1', contents: [first.fga]}, {contents: [second.fga]}]\n").expect("explicit fields"),
        ModFile {
            schema: ModFileStringProperty { value: "1.2".into(), line: 0, column: 8 },
            contents: ModFileArrayProperty {
                value: vec![ModFileStringProperty { value: "direct.fga".into(), line: 1, column: 11 }],
                line: 1,
                column: 10,
            },
        }
    );
}

/// Recursive merge aliases identify the anchor that contains itself.
#[test]
fn recursive_merge_alias() {
    let errors = parse_mod_file("&root {schema: '1.2', contents: [], <<: *root}\n")
        .expect_err("recursive merge");
    assert_eq!(
        errors.to_string(),
        "yaml: anchor 'root' value contains itself"
    );
}
