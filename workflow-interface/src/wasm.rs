use crate::{TimerResult, UnblockEvent, WfExitValue, WorkflowResult};
use prost::Message;
use serde_bytes::ByteBuf;
use std::fmt::Debug;
use temporal_sdk_core_protos::coresdk::workflow_commands::{workflow_command, WorkflowCommand};
use temporal_wasm_workflow_binding::{WasmCmdRequest, WasmUnblock, WasmWfResult};

impl From<WasmUnblock> for UnblockEvent {
    fn from(ub: WasmUnblock) -> Self {
        match ub {
            WasmUnblock::Timer(id, res) => UnblockEvent::Timer(id, res.into()),
        }
    }
}

impl From<UnblockEvent> for WasmUnblock {
    fn from(ub: UnblockEvent) -> Self {
        match ub {
            UnblockEvent::Timer(id, res) => WasmUnblock::Timer(id, res.into()),
            _ => unimplemented!(),
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
impl From<TimerResult> for temporal_wasm_workflow_binding::TimerResult {
    fn from(tr: TimerResult) -> Self {
        match tr {
            TimerResult::Cancelled => temporal_wasm_workflow_binding::TimerResult::Cancelled,
            TimerResult::Fired => temporal_wasm_workflow_binding::TimerResult::Fired,
        }
    }
}

pub fn to_wasm_result<T: Debug>(res: WorkflowResult<T>) -> WasmWfResult<T> {
    match res {
        Ok(v) => match v {
            WfExitValue::Normal(v) => WasmWfResult::Ok(v),
            _ => unimplemented!(),
        },
        Err(e) => WasmWfResult::Error(e.to_string()),
    }
}
pub fn from_wasm_result<T: Debug>(res: WasmWfResult<T>) -> WorkflowResult<T> {
    match res {
        WasmWfResult::Ok(v) => WorkflowResult::Ok(v.into()),
        WasmWfResult::Error(str) => WorkflowResult::Err(anyhow::Error::msg(str)),
    }
}

pub fn encode_wf_cmd(cmd: workflow_command::Variant) -> WasmCmdRequest {
    let cmd = WorkflowCommand { variant: Some(cmd) };
    let mut bb = Vec::new();
    cmd.encode(&mut bb).expect("Encoding works");
    WasmCmdRequest {
        wf_cmd_variant_proto: ByteBuf::from(bb),
    }
}
