use fp_bindgen::prelude::*;
use fp_bindgen::types::CargoDependency;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serializable)]
pub enum WasmWfResult<T> {
    Ok(T),
    Error(String),
}

// Required even though there are none (yet)
fp_import! {}

// Oddly, `()` is not serializable yet. Just use a string for now.
fp_export! {
    async fn invoke_workflow() -> WasmWfResult<String>;
}

const VERSION: &str = "0.1.0";
const AUTHORS: &str = r#"["Spencer Judge <spencer@temporal.io>"]"#;
const NAME: &str = "temporal-wasm-workflow-binding";
fn main() {
    for bindings_type in [
        BindingsType::RustPlugin(RustPluginConfig {
            name: NAME,
            authors: AUTHORS,
            version: VERSION,
            dependencies: BTreeMap::from([(
                "fp-bindgen-support",
                CargoDependency {
                    version: Some("1.0.0"),
                    features: BTreeSet::from(["async", "guest"]),
                    ..CargoDependency::default()
                },
            )]),
        }),
        BindingsType::RustWasmerRuntime,
    ] {
        let output_path = format!("bindings/{}", bindings_type);

        fp_bindgen!(BindingConfig {
            bindings_type,
            path: &output_path,
        });
        println!("Generated bindings written to `{}/`.", output_path);
    }
}
