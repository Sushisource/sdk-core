#![warn(missing_docs)] // error if there are missing docs

//! This crate defines an alpha-stage Temporal Rust SDK.
//!
//! Currently defining activities and running an activity-only worker is the most stable code.
//! Workflow definitions exist and running a workflow worker works, but the API is still very
//! unstable.
//!
//! An example of running an activity worker:
//! ```no_run
//! use std::{sync::Arc, str::FromStr};
//! use temporal_sdk::{sdk_client_options, Worker};
//! use temporal_sdk_core::{init_worker, Url};
//! use temporal_sdk_core_api::worker::{WorkerConfig, WorkerConfigBuilder};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let server_options = sdk_client_options(Url::from_str("http://localhost:7233")?).build()?;
//!     let client = server_options.connect("my_namespace", None).await?;
//!     let worker_config = WorkerConfigBuilder::default().build()?;
//!     let core_worker = init_worker(worker_config, client);
//!
//!     let mut worker = Worker::new_from_core(Arc::new(core_worker), "task_queue");
//!     worker.register_activity(
//!         "echo_activity",
//!         |echo_me: String| async move { Ok(echo_me) },
//!     );
//!     worker.run().await?;
//!     Ok(())
//! }
//! ```

#[macro_use]
extern crate tracing;

mod conversions;
pub mod interceptors;
mod payload_converter;
#[cfg(feature = "wasm")]
mod wasm;
mod workflow_future;

pub use temporal_workflow_interface::{
    ActivityOptions, CancellableFuture, ChildWorkflowOptions, LocalActivityOptions, Signal,
    SignalWorkflowOptions, WfContext, WfExitValue, WorkflowResult,
};

use crate::{
    interceptors::WorkerInterceptor,
    wasm::{wasm_init, WasmWorkflow},
};
use anyhow::{anyhow, bail};
use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use once_cell::sync::OnceCell;
use std::{
    collections::HashMap,
    fmt::{Debug, Display, Formatter},
    future::Future,
    sync::Arc,
};
use temporal_client::ClientOptionsBuilder;
use temporal_sdk_core::Url;
use temporal_sdk_core_api::{
    errors::{PollActivityError, PollWfError},
    Worker as CoreWorker,
};
use temporal_sdk_core_protos::{
    coresdk::{
        activity_result::ActivityExecutionResult,
        activity_task::{activity_task, ActivityTask},
        common::Payload,
        workflow_activation::{
            workflow_activation_job::Variant, WorkflowActivation, WorkflowActivationJob,
        },
        workflow_completion::WorkflowActivationCompletion,
        ActivityTaskCompletion, AsJsonPayloadExt, FromJsonPayloadExt,
    },
    TaskToken,
};
use tokio::{
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        watch,
        watch::Receiver,
    },
    task::JoinError,
};
use tokio_util::sync::CancellationToken;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns a [ClientOptionsBuilder] with required fields set to appropriate values
/// for the Rust SDK.
pub fn sdk_client_options(url: impl Into<Url>) -> ClientOptionsBuilder {
    let mut builder = ClientOptionsBuilder::default();
    builder
        .target_url(url)
        .client_name("rust-sdk".to_string())
        .client_version(VERSION.to_string())
        .worker_binary_id(binary_id().to_string());

    builder
}

/// A worker that can poll for and respond to workflow tasks by using [WorkflowFunction]s,
/// and activity tasks by using [ActivityFunction]s
pub struct Worker {
    common: CommonWorker,
    workflow_half: WorkflowHalf,
    activity_half: ActivityHalf,
}

struct CommonWorker {
    worker: Arc<dyn CoreWorker>,
    task_queue: String,
    worker_interceptor: Option<Box<dyn WorkerInterceptor>>,
}

struct WorkflowHalf {
    /// Maps run id to the driver
    workflows: HashMap<String, UnboundedSender<WorkflowActivation>>,
    /// Maps workflow type to the function for executing workflow runs with that type
    workflow_fns: HashMap<String, WorkflowFunction>,
    /// Maps workflow type to the wasm blob for executing workflow runs with that type
    #[cfg(feature = "wasm")]
    workflow_wasm_blobs: HashMap<String, WasmWorkflow>,
    /// Handles for each spawned workflow run are inserted here to be cleaned up when all runs
    /// are finished
    join_handles: FuturesUnordered<BoxFuture<'static, Result<WorkflowResult<()>, JoinError>>>,
}

struct ActivityHalf {
    /// Maps activity type to the function for executing activities of that type
    activity_fns: HashMap<String, ActivityFunction>,
    task_tokens_to_cancels: HashMap<TaskToken, CancellationToken>,
}

impl Worker {
    #[doc(hidden)]
    /// Create a new rust worker from a core worker
    pub fn new_from_core(worker: Arc<dyn CoreWorker>, task_queue: impl Into<String>) -> Self {
        Self {
            common: CommonWorker {
                worker,
                task_queue: task_queue.into(),
                worker_interceptor: None,
            },
            workflow_half: WorkflowHalf {
                workflows: Default::default(),
                workflow_fns: Default::default(),
                #[cfg(feature = "wasm")]
                workflow_wasm_blobs: Default::default(),
                join_handles: FuturesUnordered::new(),
            },
            activity_half: ActivityHalf {
                activity_fns: Default::default(),
                task_tokens_to_cancels: Default::default(),
            },
        }
    }

    /// Returns the task queue name this worker polls on
    pub fn task_queue(&self) -> &str {
        &self.common.task_queue
    }

    /// Return a handle that can be used to initiate shutdown.
    /// TODO: Doc better after shutdown changes
    pub fn shutdown_handle(&self) -> impl Fn() {
        let w = self.common.worker.clone();
        move || w.initiate_shutdown()
    }

    /// Register a Workflow function to invoke when the Worker is asked to run a workflow of
    /// `workflow_type`
    pub fn register_wf<F: Into<WorkflowFunction>>(
        &mut self,
        workflow_type: impl Into<String>,
        wf_function: F,
    ) {
        self.workflow_half
            .workflow_fns
            .insert(workflow_type.into(), wf_function.into());
    }

    /// Register a wasm blob as a workflow function for the given workflow type.
    /// TODO: Wasm blob should actually define internal mapping of types->impls that can be queried
    ///  on blob registration.
    #[cfg(feature = "wasm")]
    pub fn register_wasm_wf(&mut self, workflow_type: impl Into<String>, wasm_bytes: &[u8]) {
        let ww = wasm_init(wasm_bytes).unwrap();
        self.workflow_half
            .workflow_wasm_blobs
            .insert(workflow_type.into(), ww);
    }

    /// TODO: Delete me once working with workflow future
    pub async fn wasm_test(&self, workflow_type: &str) {
        let ww = self
            .workflow_half
            .workflow_wasm_blobs
            .get(workflow_type)
            .unwrap();
        ww.start().await;
    }

    /// Register an Activity function to invoke when the Worker is asked to run an activity of
    /// `activity_type`
    pub fn register_activity<A, R>(
        &mut self,
        activity_type: impl Into<String>,
        act_function: impl IntoActivityFunc<A, R>,
    ) {
        self.activity_half.activity_fns.insert(
            activity_type.into(),
            ActivityFunction {
                act_func: act_function.into_activity_fn(),
            },
        );
    }

    /// Runs the worker. Eventually resolves after the worker has been explicitly shut down,
    /// or may return early with an error in the event of some unresolvable problem.
    pub async fn run(&mut self) -> Result<(), anyhow::Error> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let pollers = async move {
            let (common, wf_half, act_half) = self.split_apart();
            let (completions_tx, mut completions_rx) = unbounded_channel();
            let (wf_poll_res, act_poll_res) = tokio::join!(
                // Workflow polling loop
                async {
                    loop {
                        info!("Polling");
                        let activation = match common.worker.poll_workflow_activation().await {
                            Err(PollWfError::ShutDown) => {
                                break Result::<_, anyhow::Error>::Ok(());
                            }
                            o => o?,
                        };
                        wf_half
                            .workflow_activation_handler(
                                common,
                                &shutdown_rx,
                                &completions_tx,
                                &mut completions_rx,
                                activation,
                            )
                            .await?;
                    }
                },
                // Only poll on the activity queue if activity functions have been registered. This
                // makes tests which use mocks dramatically more manageable.
                async {
                    let mut shutdown_rx = shutdown_rx.clone();
                    if !act_half.activity_fns.is_empty() {
                        loop {
                            tokio::select! {
                                activity = common.worker.poll_activity_task() => {
                                    if matches!(activity, Err(PollActivityError::ShutDown)) {
                                        break;
                                    }
                                    act_half.activity_task_handler(common.worker.clone(),
                                                                   activity?)?;
                                },
                                _ = shutdown_rx.changed() => { break }
                            }
                        }
                    };
                    Result::<_, anyhow::Error>::Ok(())
                }
            );
            wf_poll_res?;
            // TODO: Activity loop errors don't show up until wf loop exits/errors
            act_poll_res?;
            Result::<_, anyhow::Error>::Ok(self)
        };

        let myself = pollers.await?;
        info!("Polling loop exited");
        let _ = shutdown_tx.send(true);
        while let Some(h) = myself.workflow_half.join_handles.next().await {
            h??;
        }
        myself.common.worker.shutdown().await;
        myself.workflow_half.workflows.clear();
        Ok(())
    }

    /// Set a [WorkerInterceptor]
    pub fn set_worker_interceptor(&mut self, interceptor: Box<dyn WorkerInterceptor>) {
        self.common.worker_interceptor = Some(interceptor);
    }

    /// Turns this rust worker into a new worker with all the same workflows and activities
    /// registered, but with a new underlying core worker. Can be used to swap the worker for
    /// a replay worker, change task queues, etc.
    pub fn with_new_core_worker(&mut self, new_core_worker: Arc<dyn CoreWorker>) {
        self.common.worker = new_core_worker;
    }

    fn split_apart(&mut self) -> (&mut CommonWorker, &mut WorkflowHalf, &mut ActivityHalf) {
        (
            &mut self.common,
            &mut self.workflow_half,
            &mut self.activity_half,
        )
    }
}

impl WorkflowHalf {
    async fn workflow_activation_handler(
        &mut self,
        common: &CommonWorker,
        shutdown_rx: &Receiver<bool>,
        completions_tx: &UnboundedSender<WorkflowActivationCompletion>,
        completions_rx: &mut UnboundedReceiver<WorkflowActivationCompletion>,
        activation: WorkflowActivation,
    ) -> Result<(), anyhow::Error> {
        // If the activation is to start a workflow, create a new workflow driver for it,
        // using the function associated with that workflow id
        if let Some(WorkflowActivationJob {
            variant: Some(Variant::StartWorkflow(sw)),
        }) = activation.jobs.get(0)
        {
            let workflow_type = &sw.workflow_type;
            let wf_function = self
                .workflow_fns
                .get(workflow_type)
                .ok_or_else(|| anyhow!("Workflow type {workflow_type} not found"))?;

            let (wff, activations) = wf_function.start_workflow(
                common.worker.get_config().namespace.clone(),
                common.task_queue.clone(),
                // NOTE: Don't clone args if this gets ported to be a non-test rust worker
                sw.arguments.clone(),
                completions_tx.clone(),
            );
            let mut shutdown_rx = shutdown_rx.clone();
            let jh = tokio::spawn(async move {
                tokio::select! {
                    r = wff => r,
                    _ = shutdown_rx.changed() => Ok(WfExitValue::Evicted)
                }
            });
            self.workflows
                .insert(activation.run_id.clone(), activations);
            self.join_handles.push(jh.boxed());
        }

        // The activation is expected to apply to some workflow we know about. Use it to
        // unblock things and advance the workflow.
        if let Some(tx) = self.workflows.get_mut(&activation.run_id) {
            tx.send(activation)
                .expect("Workflow should exist if we're sending it an activation");
        } else {
            bail!("Got activation for unknown workflow");
        };

        let completion = completions_rx.recv().await.expect("No workflows left?");
        if let Some(ref i) = common.worker_interceptor {
            i.on_workflow_activation_completion(&completion);
        }
        common
            .worker
            .complete_workflow_activation(completion)
            .await?;
        Ok(())
    }
}

tokio::task_local! {
    // This works, but maybe just passing a context object for activities like WFs is better
    static ACT_CANCEL_TOK: CancellationToken
}

/// Returns a future the completes if and when the activity this was called inside has been
/// cancelled
pub async fn act_cancelled() {
    ACT_CANCEL_TOK.with(|ct| ct.clone()).cancelled().await
}

/// Returns true if this activity has already been cancelled
pub fn act_is_cancelled() -> bool {
    ACT_CANCEL_TOK.with(|ct| ct.is_cancelled())
}

impl ActivityHalf {
    /// Spawns off a task to handle the provided activity task
    fn activity_task_handler(
        &mut self,
        worker: Arc<dyn CoreWorker>,
        activity: ActivityTask,
    ) -> Result<(), anyhow::Error> {
        match activity.variant {
            Some(activity_task::Variant::Start(start)) => {
                let act_fn = self
                    .activity_fns
                    .get(&start.activity_type)
                    .ok_or_else(|| {
                        anyhow!(
                            "No function registered for activity type {}",
                            start.activity_type
                        )
                    })?
                    .clone();
                let ct = CancellationToken::new();
                self.task_tokens_to_cancels
                    .insert(activity.task_token.clone().into(), ct.clone());

                tokio::spawn(ACT_CANCEL_TOK.scope(ct, async move {
                    let mut inputs = start.input;
                    let arg = inputs.pop().unwrap_or_default();
                    let output = (&act_fn.act_func)(arg).await;
                    let result = match output {
                        Ok(res) => ActivityExecutionResult::ok(res),
                        Err(err) => match err.downcast::<ActivityCancelledError>() {
                            Ok(ce) => ActivityExecutionResult::cancel_from_details(ce.details),
                            Err(other_err) => ActivityExecutionResult::fail(other_err.into()),
                        },
                    };
                    worker
                        .complete_activity_task(ActivityTaskCompletion {
                            task_token: activity.task_token,
                            result: Some(result),
                        })
                        .await?;
                    Result::<_, anyhow::Error>::Ok(())
                }));
            }
            Some(activity_task::Variant::Cancel(_)) => {
                if let Some(ct) = self.task_tokens_to_cancels.get(&activity.task_token.into()) {
                    ct.cancel();
                }
            }
            None => bail!("Undefined activity task variant"),
        }
        Ok(())
    }
}

type WfFunc = dyn Fn(WfContext) -> BoxFuture<'static, WorkflowResult<()>> + Send + Sync + 'static;

/// The user's async function / workflow code
pub struct WorkflowFunction {
    wf_func: Box<WfFunc>,
}

impl<F, Fut> From<F> for WorkflowFunction
where
    F: Fn(WfContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = WorkflowResult<()>> + Send + 'static,
{
    fn from(wf_func: F) -> Self {
        Self::new(wf_func)
    }
}

impl WorkflowFunction {
    /// Build a workflow function from a closure or function pointer which accepts a [WfContext]
    pub fn new<F, Fut>(wf_func: F) -> Self
    where
        F: Fn(WfContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = WorkflowResult<()>> + Send + 'static,
    {
        Self {
            wf_func: Box::new(move |ctx: WfContext| wf_func(ctx).boxed()),
        }
    }
}

type BoxActFn =
    Arc<dyn Fn(Payload) -> BoxFuture<'static, Result<Payload, anyhow::Error>> + Send + Sync>;
/// Container for user-defined activity functions
#[derive(Clone)]
pub struct ActivityFunction {
    act_func: BoxActFn,
}

/// Return this error to indicate your activity is cancelling
#[derive(Debug, Default)]
pub struct ActivityCancelledError {
    details: Option<Payload>,
}
impl std::error::Error for ActivityCancelledError {}
impl Display for ActivityCancelledError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Activity cancelled")
    }
}

/// Closures / functions which can be turned into activity functions implement this trait
pub trait IntoActivityFunc<Args, Res> {
    /// Consume the closure or fn pointer and turned it into a boxed activity function
    fn into_activity_fn(self) -> BoxActFn;
}

impl<A, Rf, R, F> IntoActivityFunc<A, Rf> for F
where
    F: (Fn(A) -> Rf) + Sync + Send + 'static,
    A: FromJsonPayloadExt + Send,
    Rf: Future<Output = Result<R, anyhow::Error>> + Send + 'static,
    R: AsJsonPayloadExt,
{
    fn into_activity_fn(self) -> BoxActFn {
        let wrapper = move |input: Payload| {
            // Some minor gymnastics are required to avoid needing to clone the function
            match A::from_json_payload(&input) {
                Ok(deser) => (self)(deser)
                    .map(|r| r.map(|r| r.as_json_payload())?)
                    .boxed(),
                Err(e) => async move { Err(e.into()) }.boxed(),
            }
        };
        Arc::new(wrapper)
    }
}

/// Reads own binary, hashes it, and returns b64 str version of that hash
fn binary_id() -> &'static str {
    use sha2::{Digest, Sha256};
    use std::{env, fs, io};

    static INSTANCE: OnceCell<String> = OnceCell::new();
    INSTANCE.get_or_init(|| {
        let exe_path = env::current_exe().expect("Cannot read own binary to determine binary id");
        let mut exe_file =
            fs::File::open(exe_path).expect("Cannot read own binary to determine binary id");
        let mut hasher = Sha256::new();
        io::copy(&mut exe_file, &mut hasher).expect("Copying data into binary hasher works");
        let hash = hasher.finalize();
        base64::encode(hash)
    })
}
