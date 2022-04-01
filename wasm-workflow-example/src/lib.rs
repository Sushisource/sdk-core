use crossbeam::channel::Receiver;
use std::{cell::RefCell, collections::HashMap, panic, sync::Once, time::Duration};
use temporal_sdk_core_protos::coresdk::workflow_commands::workflow_command;
use temporal_wasm_workflow_binding::*;
use temporal_workflow_interface::{
    cmd_id_from_variant,
    wasm::{convert_result, encode_cmd_variant},
    CommandID, RustWfCmd, UnblockEvent, WfContext, WorkflowResult,
};
use tokio::sync::{oneshot, watch};

// TODO: This should (except actual wf definition) be lifted out into either the interface crate
//  behind a flag, or another crate specifically for wasm workflows or something. Flag better.

// TODO: Eventually want a different context per workflow to support multiple workflows in
//   one wasm blob
thread_local! {
    static UNLBOCK_MAP: RefCell<HashMap<CommandID, WFCommandFutInfo>> = RefCell::new(HashMap::new());
    static CMD_RCV: RefCell<Option<Receiver<RustWfCmd>>> = RefCell::new(None);
}

fn init_panic_hook() {
    static SET_HOOK: Once = Once::new();
    SET_HOOK.call_once(|| {
        panic::set_hook(Box::new(|info| log(info.to_string())));
    });
}

#[fp_export_impl(temporal_wasm_workflow_binding)]
async fn invoke_workflow(input: WasmWfInput) -> WasmWfResult<String> {
    init_panic_hook();

    // TODO: Save tx side of cancel
    let (cancel_tx, cancel_rx) = watch::channel(input.is_cancelled);
    let (new_ctx, wf_cmd_rx) = WfContext::new(input.namespace, input.task_queue, vec![], cancel_rx);
    CMD_RCV.with(|cmd_rcv| {
        *cmd_rcv.borrow_mut() = Some(wf_cmd_rx);
    });

    let wf_fut = timer_wf(new_ctx);
    convert_result(wf_fut.await)
}

#[fp_export_impl(temporal_wasm_workflow_binding)]
fn gather_commands() -> Vec<WasmWfCmd> {
    init_panic_hook();

    let mut cmds = vec![];
    CMD_RCV.with(|rcv| {
        let mut borrowed = rcv.borrow_mut();
        let rcv = (*borrowed).as_mut().expect("Channel exists");
        while let Ok(cmd) = rcv.try_recv() {
            let cmd = match cmd {
                RustWfCmd::NewCmd(c) => {
                    add_cmd_to_unblock_map(&c.cmd, c.unblocker);
                    WasmWfCmd::NewCmd(encode_cmd_variant(c.cmd))
                }
                _ => unimplemented!(),
            };
            cmds.push(cmd);
        }
    });
    cmds
}

#[fp_export_impl(temporal_wasm_workflow_binding)]
fn unblock(dat: WasmUnblock) -> u32 {
    UNLBOCK_MAP.with(|map| {
        let id = match dat {
            WasmUnblock::Timer(seq, _) => CommandID::Timer(seq),
        };
        let unblocker = map.borrow_mut().remove(&id);
        unblocker
            .expect("TODO: Return something instead of panic")
            .unblocker
            .send(dat.into())
            .expect("Receive half of unblock channel must exist");
    });
    0
}

struct WFCommandFutInfo {
    unblocker: oneshot::Sender<UnblockEvent>,
}

fn add_cmd_to_unblock_map(
    variant: &workflow_command::Variant,
    unblocker: oneshot::Sender<UnblockEvent>,
) {
    let cmd_id = cmd_id_from_variant(variant);
    let info = WFCommandFutInfo { unblocker };
    UNLBOCK_MAP.with(|map| {
        map.borrow_mut().insert(cmd_id, info);
    });
}

async fn timer_wf(command_sink: WfContext) -> WorkflowResult<String> {
    command_sink.timer(Duration::from_secs(1)).await;
    Ok("Yay!".to_string().into())
}
