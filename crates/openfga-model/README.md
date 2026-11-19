# openfga-model

[![CI](https://github.com/LucaCappelletti94/openfga-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/openfga-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/openfga-rs/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/openfga-rs)
[![crates.io](https://img.shields.io/crates/v/openfga-model.svg)](https://crates.io/crates/openfga-model)
[![docs.rs](https://docs.rs/openfga-model/badge.svg)](https://docs.rs/openfga-model)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/LucaCappelletti94/openfga-rs/blob/main/LICENSE)

Not published yet, so the crates.io and docs.rs badges resolve after the first release.

The [OpenFGA](https://openfga.dev) authorization model as Rust types, generated from [`authzmodel.proto`](https://github.com/openfga/api/blob/main/openfga/v1/authzmodel.proto). Each type reads and writes the protobuf binary of the gRPC API and the JSON of the HTTP API and `fga model transform`. The crate is `no_std` and needs only `alloc`.

```rust
use openfga_model::{AuthorizationModel, userset::Userset};
use prost::Message;

let model: AuthorizationModel = serde_json::from_str(
    r#"{
        "schema_version": "1.1",
        "type_definitions": [
            { "type": "user" },
            {
                "type": "document",
                "relations": { "viewer": { "this": {} } },
                "metadata": {
                    "relations": {
                        "viewer": { "directly_related_user_types": [{ "type": "user" }] }
                    }
                }
            }
        ]
    }"#,
)?;
let viewer = &model.type_definitions[1].relations["viewer"];
assert!(matches!(viewer.userset, Some(Userset::This(_))));

let bytes = model.encode_to_vec();
assert_eq!(AuthorizationModel::decode(bytes.as_slice())?, model);
# Ok::<(), Box<dyn std::error::Error>>(())
```
