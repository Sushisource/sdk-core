use fp_bindgen::{prelude::*, types::CargoDependency};
use serde_bytes::ByteBuf;
use std::collections::{BTreeMap, BTreeSet};

// TODO: Some of these types can be shared between native/wasm by feature-flagging whether
//   or not they derive all the serialization goop.

#[derive(Serializable)]
pub struct WasmWfInput {
    namespace: String,
    task_queue: String,
    // TODO: Args
    is_cancelled: bool,
}

#[derive(Serializable)]
pub enum WasmWfResult<T> {
    Ok(T),
    Error(String),
}

#[derive(Serializable)]
pub enum WasmWfCmd {
    NewCmd(WasmCmdRequest),
}

#[derive(Serializable)]
pub struct WasmCmdRequest {
    /// A serialized workflow_command::Variant
    pub wf_cmd_variant_proto: ByteBuf,
}

#[derive(Serializable)]
pub enum WasmUnblock {
    Timer(u32, TimerResult),
}

/// Result of awaiting on a timer
#[derive(Debug, Copy, Clone, Serializable)]
pub enum TimerResult {
    /// The timer was cancelled
    Cancelled,
    /// The timer elapsed and fired
    Fired,
}

// Required even though there are none (yet)
fp_import! {}

// Oddly, `()` is not serializable yet. Just use a string for now.
fp_export! {
    async fn invoke_workflow(input: WasmWfInput) -> WasmWfResult<String>;
    fn gather_commands() -> Vec<WasmWfCmd>;
    // Return value currently pointless. Macros break with no return value.
    fn unblock(dat: WasmUnblock) -> u32;
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
