mod workflow_context;

use crate::workflow_context::{ChildWfCommon, PendingChildWorkflow, SignalData};
use std::fmt::Debug;
use temporal_sdk_core_protos::{
    coresdk::{
        activity_result::ActivityResolution,
        child_workflow::ChildWorkflowResult,
        common::NamespacedWorkflowExecution,
        workflow_activation::resolve_child_workflow_execution_start::Status as ChildWorkflowStartStatus,
        workflow_commands::{workflow_command, ContinueAsNewWorkflowExecution},
    },
    temporal::api::failure::v1::Failure,
};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

/// The result of running a workflow
pub type WorkflowResult<T> = Result<WfExitValue<T>, anyhow::Error>;

/// Workflow functions may return these values when exiting
#[derive(Debug, derive_more::From)]
pub enum WfExitValue<T: Debug> {
    /// Continue the workflow as a new execution
    #[from(ignore)]
    ContinueAsNew(Box<ContinueAsNewWorkflowExecution>),
    /// Confirm the workflow was cancelled (can be automatic in a more advanced iteration)
    #[from(ignore)]
    Cancelled,
    /// The run was evicted
    #[from(ignore)]
    Evicted,
    /// Finish with a result
    Normal(T),
}

#[derive(Debug)]
enum UnblockEvent {
    Timer(u32, TimerResult),
    Activity(u32, Box<ActivityResolution>),
    WorkflowStart(u32, Box<ChildWorkflowStartStatus>),
    WorkflowComplete(u32, Box<ChildWorkflowResult>),
    SignalExternal(u32, Option<Failure>),
    CancelExternal(u32, Option<Failure>),
}

trait Unblockable {
    type OtherDat;

    fn unblock(ue: UnblockEvent, od: Self::OtherDat) -> Self;
}

impl Unblockable for TimerResult {
    type OtherDat = ();
    fn unblock(ue: UnblockEvent, _: Self::OtherDat) -> Self {
        match ue {
            UnblockEvent::Timer(_, result) => result,
            _ => panic!("Invalid unblock event for timer"),
        }
    }
}

impl Unblockable for ActivityResolution {
    type OtherDat = ();
    fn unblock(ue: UnblockEvent, _: Self::OtherDat) -> Self {
        match ue {
            UnblockEvent::Activity(_, result) => *result,
            _ => panic!("Invalid unblock event for activity"),
        }
    }
}

impl Unblockable for PendingChildWorkflow {
    // Other data here is workflow id
    type OtherDat = ChildWfCommon;
    fn unblock(ue: UnblockEvent, od: Self::OtherDat) -> Self {
        match ue {
            UnblockEvent::WorkflowStart(_, result) => Self {
                status: *result,
                common: od,
            },
            _ => panic!("Invalid unblock event for child workflow start"),
        }
    }
}

impl Unblockable for ChildWorkflowResult {
    type OtherDat = ();
    fn unblock(ue: UnblockEvent, _: Self::OtherDat) -> Self {
        match ue {
            UnblockEvent::WorkflowComplete(_, result) => *result,
            _ => panic!("Invalid unblock event for child workflow complete"),
        }
    }
}

impl Unblockable for SignalExternalWfResult {
    type OtherDat = ();
    fn unblock(ue: UnblockEvent, _: Self::OtherDat) -> Self {
        match ue {
            UnblockEvent::SignalExternal(_, maybefail) => {
                maybefail.map_or(Ok(SignalExternalOk), Err)
            }
            _ => panic!("Invalid unblock event for signal external workflow result"),
        }
    }
}

impl Unblockable for CancelExternalWfResult {
    type OtherDat = ();
    fn unblock(ue: UnblockEvent, _: Self::OtherDat) -> Self {
        match ue {
            UnblockEvent::CancelExternal(_, maybefail) => {
                maybefail.map_or(Ok(CancelExternalOk), Err)
            }
            _ => panic!("Invalid unblock event for signal external workflow result"),
        }
    }
}

/// Result of awaiting on a timer
#[derive(Debug, Copy, Clone)]
pub enum TimerResult {
    /// The timer was cancelled
    Cancelled,
    /// The timer elapsed and fired
    Fired,
}

/// Successful result of sending a signal to an external workflow
pub struct SignalExternalOk;
/// Result of awaiting on sending a signal to an external workflow
pub type SignalExternalWfResult = Result<SignalExternalOk, Failure>;

/// Successful result of sending a cancel request to an external workflow
pub struct CancelExternalOk;
/// Result of awaiting on sending a cancel request to an external workflow
pub type CancelExternalWfResult = Result<CancelExternalOk, Failure>;

/// Identifier for cancellable operations
#[derive(Debug, Clone)]
pub enum CancellableID {
    /// Timer sequence number
    Timer(u32),
    /// Activity sequence number
    Activity(u32),
    /// Activity sequence number
    LocalActivity(u32),
    /// Start child sequence number
    ChildWorkflow(u32),
    /// Signal workflow
    SignalExternalWorkflow(u32),
    /// An external workflow identifier as may have been created by a started child workflow
    ExternalWorkflow {
        /// Sequence number which will be used for the cancel command
        seqnum: u32,
        /// Identifying information about the workflow to be cancelled
        execution: NamespacedWorkflowExecution,
        /// Set to true if this workflow is a child of the issuing workflow
        only_child: bool,
    },
}

#[derive(derive_more::From)]
#[allow(clippy::large_enum_variant)]
enum RustWfCmd {
    #[from(ignore)]
    Cancel(CancellableID),
    ForceWFTFailure(anyhow::Error),
    NewCmd(CommandCreateRequest),
    NewNonblockingCmd(workflow_command::Variant),
    SubscribeChildWorkflowCompletion(CommandSubscribeChildWorkflowCompletion),
    SubscribeSignal(String, UnboundedSender<SignalData>),
}

struct CommandCreateRequest {
    cmd: workflow_command::Variant,
    unblocker: oneshot::Sender<UnblockEvent>,
}

struct CommandSubscribeChildWorkflowCompletion {
    seq: u32,
    unblocker: oneshot::Sender<UnblockEvent>,
}
