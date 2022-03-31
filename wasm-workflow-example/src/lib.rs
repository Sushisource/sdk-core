use fp_bindgen::prelude::*;
use std::time::Duration;
use temporal_workflow_interface::{WfContext, WorkflowResult};

async fn timer_wf(command_sink: WfContext) -> WorkflowResult<()> {
    command_sink.timer(Duration::from_secs(1)).await;
    Ok(().into())
}
