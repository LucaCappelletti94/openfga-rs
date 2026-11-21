//! Modular transformation conformance.
use openfga_language::{ModuleErrorKind, ModuleFile, merge_modules};
use openfga_model::AuthorizationModel;
use serde_json::Value;
use std::{fs, path::Path};

/// Compares all modular fixtures after removing validation-only findings.
#[test]
fn module_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/openfga-language/tests/data/transformer-module");
    for entry in fs::read_dir(root).unwrap() {
        let directory = entry.unwrap().path();
        if !directory.join("module").is_dir() || directory.join("test.skip").exists() {
            continue;
        }
        let expected_errors = directory.join("expected_errors.json");
        let expected: Vec<Value> = if expected_errors.exists() {
            serde_json::from_str::<Vec<Value>>(&fs::read_to_string(expected_errors).unwrap())
                .unwrap()
                .into_iter()
                .filter(|error| error["metadata"]["errorType"].is_null())
                .collect()
        } else {
            Vec::new()
        };
        if expected.is_empty() && !directory.join("authorization-model.json").exists() {
            continue;
        }
        let mut paths: Vec<_> = fs::read_dir(directory.join("module"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        let sources: Vec<_> = paths
            .iter()
            .map(|path| {
                (
                    path.file_name().unwrap().to_str().unwrap().to_owned(),
                    fs::read_to_string(path).unwrap(),
                )
            })
            .collect();
        let modules: Vec<_> = sources
            .iter()
            .map(|(name, contents)| ModuleFile { name, contents })
            .collect();
        let result = merge_modules(&modules, "1.2");
        if expected.is_empty() {
            let model: AuthorizationModel = serde_json::from_str(
                &fs::read_to_string(directory.join("authorization-model.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(result.unwrap(), model, "{}", directory.display());
        } else {
            let errors = result.unwrap_err();
            assert_eq!(
                errors.errors.len(),
                expected.len(),
                "{}",
                directory.display()
            );
            assert_errors(&errors, &expected);
        }
    }
}

/// Compares positions and messages while allowing parser-specific syntax wording.
fn assert_errors(errors: &openfga_language::ModuleErrors, expected: &[Value]) {
    for (actual, expected) in errors.errors.iter().zip(expected) {
        assert_eq!(actual.file, expected["file"].as_str().unwrap());
        assert_eq!(
            actual.line.start,
            usize::try_from(expected["line"]["start"].as_u64().unwrap()).unwrap()
        );
        assert_eq!(
            actual.line.end,
            usize::try_from(expected["line"]["end"].as_u64().unwrap()).unwrap()
        );
        assert_eq!(
            actual.column.start,
            usize::try_from(expected["column"]["start"].as_u64().unwrap()).unwrap()
        );
        assert_eq!(
            actual.column.end,
            usize::try_from(expected["column"]["end"].as_u64().unwrap()).unwrap()
        );
        if expected["type"] == "syntax" {
            assert_eq!(actual.kind, ModuleErrorKind::Syntax);
        } else {
            assert_eq!(actual.kind, ModuleErrorKind::Transformation);
            assert_eq!(actual.message, expected["msg"].as_str().unwrap());
        }
    }
    if expected.iter().all(|error| error["type"] != "syntax") {
        let messages: Vec<_> = expected
            .iter()
            .map(|error| {
                format!(
                    "transformation error at line={}, column={}: {}",
                    error["line"]["start"],
                    error["column"]["start"],
                    error["msg"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(
            errors.to_string(),
            format!(
                "{} error{} occurred:\n\t* {}\n\n",
                messages.len(),
                if messages.len() == 1 { "" } else { "s" },
                messages.join("\n\t* ")
            )
        );
    }
}

/// Preserves the schema supplied by the caller.
#[test]
fn schema_version() {
    let model = merge_modules(
        &[ModuleFile {
            name: "core.fga",
            contents: "module core\n  type user",
        }],
        "1.1",
    )
    .unwrap();
    assert_eq!(model.schema_version, "1.1");
}

/// Matches declaration diagnostics observed from the modular transformer.
#[test]
fn declaration_errors() {
    for (source, expected) in [
        (
            "module core\ntype user\ntype user",
            "transformation error at line=1, column=5: duplicate type definition user",
        ),
        (
            "module core\ntype user\n  relations\n    define viewer: [user]\n    define viewer: [user]",
            "syntax error at line=4, column=11: 'viewer' is already defined in 'user'",
        ),
        (
            "module core\nextend type user\nextend type user",
            "syntax error at line=2, column=12: 'user' is already extended in file.",
        ),
        (
            "module core\ntype user\ncondition test(value: bool) { value }\ncondition test(value: bool) { value }",
            "syntax error at line=3, column=10: condition 'test' is already defined in the model",
        ),
    ] {
        let errors = merge_modules(
            &[ModuleFile {
                name: "core.fga",
                contents: source,
            }],
            "1.2",
        )
        .unwrap_err();
        assert_eq!(
            errors.to_string(),
            format!("1 error occurred:\n\t* {expected}\n\n")
        );
    }
}

/// Empty input and empty extensions retain only declared model data.
#[test]
fn empty_modules_and_extensions() {
    assert_eq!(
        merge_modules(&[], "1.2").unwrap(),
        AuthorizationModel {
            schema_version: "1.2".to_owned(),
            ..AuthorizationModel::default()
        }
    );
    let model = merge_modules(
        &[
            ModuleFile {
                name: "core.fga",
                contents: "module core\ntype user",
            },
            ModuleFile {
                name: "extension.fga",
                contents: "module extension\nextend type user",
            },
        ],
        "1.2",
    )
    .unwrap();
    let expected: AuthorizationModel = serde_json::from_str(r#"{"schema_version":"1.2","type_definitions":[{"type":"user","metadata":{"module":"core","source_info":{"file":"core.fga"}}}]}"#).unwrap();
    assert_eq!(model, expected);
}

/// Reports every missing extension in declaration order.
#[test]
fn multiple_missing_extensions() {
    let errors = merge_modules(
        &[ModuleFile {
            name: "core.fga",
            contents: "module core\nextend type user\nextend type document",
        }],
        "1.2",
    )
    .unwrap_err();
    assert_eq!(
        errors.to_string(),
        "2 errors occurred:\n\t* transformation error at line=1, column=12: extended type user does not exist\n\t* transformation error at line=2, column=12: extended type document does not exist\n\n"
    );
}

/// Nonmodular conditions retain the positioned module error.
#[test]
fn nonmodular_condition_reports_module_error_without_go_oracle() {
    let errors = merge_modules(
        &[
            ModuleFile {
                name: "model.fga",
                contents: "model\n  schema 1.1\ntype user\ncondition active(value: bool) { value }",
            },
            ModuleFile {
                name: "core.fga",
                contents: "module core\ntype user\ncondition active(value: bool) { value }",
            },
        ],
        "1.2",
    )
    .unwrap_err();
    assert_eq!(
        errors,
        openfga_language::ModuleErrors {
            errors: vec![openfga_language::ModuleError {
                message: "file is not a module".to_owned(),
                file: "model.fga".to_owned(),
                line: openfga_language::PositionRange { start: 0, end: 0 },
                column: openfga_language::PositionRange { start: 0, end: 0 },
                kind: ModuleErrorKind::Transformation,
            }],
        }
    );
}

/// Duplicate conditions and extension relations follow declaration order.
#[test]
fn merge_errors_follow_input_order() {
    let errors = merge_modules(&[
        ModuleFile {
            name: "core.fga",
            contents: "module core\ntype document\n  relations\n    define zeta: [document]\n    define alpha: [document]\ncondition zeta(value: bool) { value }\ncondition alpha(value: bool) { value }",
        },
        ModuleFile {
            name: "extend.fga",
            contents: "module extension\nextend type document\n  relations\n    define zeta: [document]\n    define alpha: [document]\ncondition zeta(value: bool) { value }\ncondition alpha(value: bool) { value }",
        },
    ], "1.2").unwrap_err();
    assert_eq!(
        errors
            .errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>(),
        [
            "duplicate condition zeta",
            "duplicate condition alpha",
            "relation zeta already exists on type document",
            "relation alpha already exists on type document",
        ]
    );
}
