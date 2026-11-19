# openfga-rs

[![CI](https://github.com/LucaCappelletti94/openfga-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/openfga-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/openfga-rs/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/openfga-rs)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/LucaCappelletti94/openfga-rs/blob/main/LICENSE)

[OpenFGA](https://openfga.dev) authorization models in Rust, `no_std`.

| Crate | What it holds |
|---|---|
| [`openfga-model`](crates/openfga-model) | The model types generated from `authzmodel.proto`, with protobuf binary and JSON encodings |

Conformance tests read the pinned [openfga/language](https://github.com/openfga/language) corpus, so clone with `--recurse-submodules`.
