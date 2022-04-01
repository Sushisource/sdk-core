use std::sync::Arc;
use temporal_sdk::Worker;
use temporal_sdk_core::api::CoreTelemetry;
use temporal_sdk_core::{init_worker, telemetry_init, WorkerConfigBuilder};
use temporal_sdk_core_test_utils::{get_integ_server_options, get_integ_telem_options};

#[tokio::main]
async fn main() {
    // TODO: Dynamic load
    let wasm_bytes =
        include_bytes!("../../target/wasm32-unknown-unknown/release/wasm_workflow_example.wasm");

    println!("Loaded {} bytes of WASM", wasm_bytes.len());

    let opts = get_integ_server_options();
    let telem_d = telemetry_init(&get_integ_telem_options()).unwrap();
    let retrying_client = opts
        .connect_no_namespace(telem_d.get_metric_meter())
        .await
        .unwrap();

    let worker = init_worker(
        WorkerConfigBuilder::default()
            .namespace("default")
            .task_queue("wasm_worker")
            .build()
            .unwrap(),
        retrying_client,
    );
    let mut worker = Worker::new_from_core(Arc::new(worker), "wasm_worker");
    worker.register_wasm_wf("wasm_wf", wasm_bytes);
}
