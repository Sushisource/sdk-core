use crate::wasm::types::{TimerResult, WasmUnblock};
use crate::wasm::{runtime::Runtime, types::WasmWfInput};
use std::time::Duration;

// use wasmer::{imports, Cranelift, Instance, Module, Store, Universal};
mod types {
    include!("../../bindings/rust-wasmer-runtime/types.rs");
}
mod runtime {
    // NOTE: All copy-pasted and modified from generated bindings.
    use super::types::*;
    use fp_bindgen_support::{
        common::mem::FatPtr,
        host::{
            errors::{InvocationError, RuntimeError},
            mem::{
                deserialize_from_slice, export_to_guest_raw, import_from_guest,
                import_from_guest_raw, serialize_to_vec,
            },
            r#async::{future::ModuleRawFuture, resolve_async_value},
            runtime::RuntimeInstanceData,
        },
    };
    use wasmer::{imports, Function, ImportObject, Instance, Module, Store, WasmerEnv};

    pub struct Runtime {
        module: Module,
        env: RuntimeInstanceData,
        instance: Instance,
    }

    impl Runtime {
        pub fn new(wasm_module: impl AsRef<[u8]>) -> Result<Self, RuntimeError> {
            let store = Self::default_store();
            let module = Module::new(&store, wasm_module)?;
            let mut env = RuntimeInstanceData::default();
            let import_object = create_import_object(module.store(), &env);
            let instance = Instance::new(&module, &import_object).unwrap();
            env.init_with_instance(&instance).unwrap();
            Ok(Self {
                module,
                env,
                instance,
            })
        }

        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
        fn default_store() -> wasmer::Store {
            let compiler = wasmer_compiler_cranelift::Cranelift::default();
            let engine = wasmer_engine_universal::Universal::new(compiler).engine();
            Store::new(&engine)
        }

        #[cfg(not(any(target_arch = "arm", target_arch = "aarch64")))]
        fn default_store() -> wasmer::Store {
            let compiler = wasmer_compiler_singlepass::Singlepass::default();
            let engine = wasmer_engine_universal::Universal::new(compiler).engine();
            Store::new(&engine)
        }

        pub fn gather_commands(&self) -> Result<Vec<WasmWfCmd>, InvocationError> {
            let result = self.gather_commands_raw();
            let result = result.map(|ref data| deserialize_from_slice(data));
            result
        }
        pub fn gather_commands_raw(&self) -> Result<Vec<u8>, InvocationError> {
            let function = self
                .instance
                .exports
                .get_native_function::<(), FatPtr>("__fp_gen_gather_commands")
                .map_err(|_| InvocationError::FunctionNotExported)?;
            let result = function.call()?;
            let result = import_from_guest_raw(&self.env, result);
            Ok(result)
        }

        pub async fn invoke_workflow(
            &self,
            input: WasmWfInput,
        ) -> Result<WasmWfResult<String>, InvocationError> {
            let input = serialize_to_vec(&input);
            let result = self.invoke_workflow_raw(input);
            let result = result.await;
            let result = result.map(|ref data| deserialize_from_slice(data));
            result
        }
        pub async fn invoke_workflow_raw(
            &self,
            input: Vec<u8>,
        ) -> Result<Vec<u8>, InvocationError> {
            let input = export_to_guest_raw(&self.env, input);
            let function = self
                .instance
                .exports
                .get_native_function::<FatPtr, FatPtr>("__fp_gen_invoke_workflow")
                .map_err(|_| InvocationError::FunctionNotExported)?;
            let result = function.call(input)?;
            let result = ModuleRawFuture::new(self.env.clone(), result).await;
            Ok(result)
        }

        pub fn unblock(&self, dat: WasmUnblock) -> Result<u32, InvocationError> {
            let dat = serialize_to_vec(&dat);
            let result = self.unblock_raw(dat);
            result
        }
        pub fn unblock_raw(&self, dat: Vec<u8>) -> Result<u32, InvocationError> {
            let dat = export_to_guest_raw(&self.env, dat);
            let function = self
                .instance
                .exports
                .get_native_function::<FatPtr, u32>("__fp_gen_unblock")
                .map_err(|_| InvocationError::FunctionNotExported)?;
            let result = function.call(dat)?;
            Ok(result)
        }
    }

    fn create_import_object(store: &Store, env: &RuntimeInstanceData) -> ImportObject {
        imports! {
           "fp" => {
               "__fp_host_resolve_async_value" => Function :: new_native_with_env (store , env . clone () , resolve_async_value) ,
               "__fp_gen_log" => Function :: new_native_with_env (store , env . clone () , _log) ,
            }
        }
    }

    pub fn _log(env: &RuntimeInstanceData, message: FatPtr) {
        let message = import_from_guest::<String>(env, message);
        super::log(message);
    }
}

pub(crate) struct WasmWorkflow {
    pub runtime: Runtime,
}

pub(crate) fn wasm_init(wasm_bytes: &[u8]) -> Result<WasmWorkflow, anyhow::Error> {
    let runtime = runtime::Runtime::new(wasm_bytes)?;
    info!("Done compiling wasm!");
    Ok(WasmWorkflow { runtime })
}

impl WasmWorkflow {
    pub async fn start(&self) {
        info!("Invoking");
        let run_fut = async {
            self.runtime
                .invoke_workflow(WasmWfInput {
                    namespace: "ns".to_string(),
                    task_queue: "tq".to_string(),
                    is_cancelled: false,
                })
                .await
                .unwrap()
        };
        let gather_fut = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let cmds = self.runtime.gather_commands().unwrap();
            info!("commands: {:?}", cmds);
            self.runtime
                .unblock(WasmUnblock::Timer(1, TimerResult::Fired))
                .unwrap();
        };

        let (res, _) = tokio::join!(run_fut, gather_fut);
        info!("wf complete, res: {:?}", res);
    }
}

// Import fns
fn log(msg: String) {
    info!("wasm: {}", msg);
}
