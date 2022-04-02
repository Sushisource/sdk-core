use std::sync::Arc;
use temporal_client::{WorkflowClientTrait, WorkflowOptions};
use temporal_sdk::Worker;
use temporal_sdk_core::protos::temporal::api::common::v1::WorkflowType;
use temporal_sdk_core::protos::temporal::api::workflowservice::v1::StartWorkflowExecutionRequest;
use temporal_sdk_core::{api::CoreTelemetry, init_worker, telemetry_init, WorkerConfigBuilder};
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
        .connect("default", telem_d.get_metric_meter())
        .await
        .unwrap();

    retrying_client
        .start_workflow(
            vec![],
            "wasm_worker".to_string(),
            "test_wasm_wf".to_string(),
            "wasm_wf".to_string(),
            WorkflowOptions::default(),
        )
        .await
        .unwrap();
    let worker = init_worker(
        WorkerConfigBuilder::default()
            .namespace("default")
            .task_queue("wasm_worker")
            .max_cached_workflows(1usize)
            .build()
            .unwrap(),
        retrying_client,
    );
    let mut worker = Worker::new_from_core(Arc::new(worker), "wasm_worker");
    worker.register_wasm_wf("wasm_wf", wasm_bytes);
    worker.run().await.unwrap();
}
