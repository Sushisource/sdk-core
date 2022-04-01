use crate::{TimerResult, UnblockEvent, WfExitValue, WorkflowResult};
use serde_bytes::ByteBuf;
use std::fmt::Debug;
use temporal_sdk_core_protos::coresdk::workflow_commands::workflow_command;
use temporal_wasm_workflow_binding::{WasmCmdRequest, WasmUnblock, WasmWfResult};

impl From<WasmUnblock> for UnblockEvent {
    fn from(ub: WasmUnblock) -> Self {
        match ub {
            WasmUnblock::Timer(id, res) => UnblockEvent::Timer(id, res.into()),
        }
    }
}

impl From<temporal_wasm_workflow_binding::TimerResult> for TimerResult {
    fn from(tr: temporal_wasm_workflow_binding::TimerResult) -> Self {
        match tr {
            temporal_wasm_workflow_binding::TimerResult::Cancelled => TimerResult::Cancelled,
            temporal_wasm_workflow_binding::TimerResult::Fired => TimerResult::Fired,
        }
    }
}

pub fn convert_result<T: Debug>(res: WorkflowResult<T>) -> WasmWfResult<T> {
    match res {
        Ok(v) => match v {
            WfExitValue::Normal(v) => WasmWfResult::Ok(v),
            _ => unimplemented!(),
        },
        Err(e) => WasmWfResult::Error(e.to_string()),
    }
}

pub fn encode_cmd_variant(cmd: workflow_command::Variant) -> WasmCmdRequest {
    let mut bb = Vec::new();
    cmd.encode(&mut bb);
    WasmCmdRequest {
        wf_cmd_variant_proto: ByteBuf::from(bb),
    }
}
