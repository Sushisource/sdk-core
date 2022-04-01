// use wasmer::{imports, Cranelift, Instance, Module, Store, Universal};
mod types {
    include!("../../bindings/rust-wasmer-runtime/types.rs");
}
mod runtime {
    include!("../../bindings/rust-wasmer-runtime/bindings.rs");
}

pub(crate) struct WasmWorkflow {}

// TODO: Break apart
pub(crate) fn wasm_test(wasm_bytes: &[u8]) -> Result<(), anyhow::Error> {
    // let store = Store::new(&Universal::new(Cranelift::default()).engine());
    //
    // info!("Compiling module...");
    // // Let's compile the Wasm module.
    // let module = Module::new(&store, wasm_bytes)?;
    //
    // // Create an empty import object.
    // let import_object = imports! {};
    //
    // info!("Instantiating module...");
    // // Let's instantiate the Wasm module.
    // let _instance = Instance::new(&module, &import_object)?;

    runtime::Runtime::new(wasm_bytes)?;
    info!("Done compiling wasm!");

    Ok(())
}
