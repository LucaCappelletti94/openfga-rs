# openfga-language

[![CI](https://github.com/LucaCappelletti94/openfga-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/openfga-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/openfga-rs/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/openfga-rs)
[![crates.io](https://img.shields.io/crates/v/openfga-language.svg)](https://crates.io/crates/openfga-language)
[![docs.rs](https://docs.rs/openfga-language/badge.svg)](https://docs.rs/openfga-language)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/LucaCappelletti94/openfga-rs/blob/main/LICENSE)

Not published yet, so the crates.io and docs.rs badges resolve after the first release.

Parse [OpenFGA](https://openfga.dev) DSL into `openfga-model` types and write single-file models as DSL with `no_std` and `alloc`.

```rust
let model = openfga_language::parse("model\n  schema 1.1\n\ntype user")?;
openfga_language::validate(&model)?;
assert_eq!(model.schema_version, "1.1");
assert_eq!(model.type_definitions[0].r#type, "user");
assert_eq!(openfga_language::parse(&openfga_language::write(&model)?)?, model);
# Ok::<(), Box<dyn std::error::Error>>(())
```
