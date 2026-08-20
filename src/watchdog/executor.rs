//! Shared execution path for accepted control operations.
//!
//! Both entry points — the core child's pipe IPC and the private control RPC —
//! must drive the *same* accept → run → settle sequence. Keeping it in one place
//! is not just deduplication: `ControlService` holds an exclusive-operation lock
//! that is only released by `mark_done`/`mark_error`. Any path that calls
//! `accept()` without eventually settling the operation wedges every subsequent
//! update/rollback/restart behind a permanent `Busy`.
//!
//! Invariant enforced here: **every accepted exclusive operation is settled**,
//! on success, failure, and panic.

use crate::config::WatchdogConfig;
use crate::error::{Result, SebasError};
use crate::watchdog::auth::{AssertionPrincipal, actor_to_principal};
use crate::watchdog::confirmation::{ConfirmationError, ConfirmationService};
use crate::watchdog::control::{
    Actor, ControlRequest, ControlResponse, ControlService, UpdateKind, UpdateTarget,
};
use crate::watchdog::control_rpc::{RpcControlResponse, RpcServiceStatus};
use crate::watchdog::updater::{UpdatePlan, UpdaterRunner};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::{info, warn};

/// What the supervisor should do once an operation settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostAction {
    /// Nothing to do (dry-run, status, or a failed operation).
    None,
    /// The core child must be restarted. `is_upgrade` tells the watchdog whether
    /// this restart launches a freshly-installed binary (so a subsequent crash
    /// *before Ready* can be classified as NewBinaryNotReady and trigger an
    /// automated rollback) vs. a plain restart of the current binary.
    RestartCore { is_upgrade: bool },
}

/// Outcome of running an operation to completion.
#[derive(Debug, Clone)]
pub struct ExecutionOutcome {
    pub operation_id: String,
    pub post_action: PostAction,
}

/// How an accepted request is carried out.
enum Execution {
    /// Run the updater subprocess with this plan.
    Updater {
        plan: UpdatePlan,
        label: &'static str,
    },
    /// Settle immediately and restart the core child. No installer work.
    RestartOnly,
    /// Settle immediately with nothing to do (Status, service queries).
    Nothing,
}

/// Owns everything needed to turn a `ControlRequest` into real work.
///
/// Cloneable so adapters (RPC handlers, IPC loop) can each hold one; all clones
/// share the same `ControlService` and restart channel.
#[derive(Clone)]
pub struct ControlExecutor {
    control: Arc<Mutex<ControlService>>,
    runner: Arc<dyn UpdaterRunner>,
    config: WatchdogConfig,
    config_path: String,
    restart_tx: mpsc::UnboundedSender<PostAction>,
    /// Single-use, short-lived confirmation grants for dangerous actions
    /// (spec §7). Shared across executor clones.
    confirmation: Arc<ConfirmationService>,
    /// Pending dangerous actions awaiting confirmation, keyed by the opaque
    /// grant token. The `(Actor, ControlRequest)` is the canonical action
    /// truth — the client only ever carries the token.
    pending: Arc<Mutex<HashMap<String, PendingControl>>>,
}

/// A dangerous action held until its confirmation token is redeemed.
#[derive(Debug, Clone)]
struct PendingControl {
    actor: Actor,
    request: ControlRequest,
}

/// TTL for dangerous-action confirmation grants (spec §7: short-lived).
const CONFIRMATION_TTL_SECS: u64 = 300;

/// A dangerous action awaiting user confirmation.
#[derive(Debug, Clone)]
pub struct ConfirmationCreated {
    pub token: String,
    pub action: String,
    pub message: String,
    pub expires_in: u64,
}

impl ControlExecutor {
    pub fn new(
        control: Arc<Mutex<ControlService>>,
        runner: Arc<dyn UpdaterRunner>,
        config: WatchdogConfig,
        config_path: String,
        restart_tx: mpsc::UnboundedSender<PostAction>,
    ) -> Self {
        Self {
            control,
            runner,
            config,
            config_path,
            restart_tx,
            confirmation: Arc::new(ConfirmationService::new()),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn control(&self) -> &Arc<Mutex<ControlService>> {
        &self.control
    }

    /// Accept a request and run it to completion, awaiting the result.
    ///
    /// Used by callers that want to report the final outcome inline (the core
    /// child's IPC path streams progress back over the pipe).
    pub async fn submit_blocking(
        &self,
        actor: Actor,
        request: ControlRequest,
    ) -> Result<ExecutionOutcome> {
        let operation_id = self.accept(actor, request.clone()).await?;
        let outcome = self.run_accepted(operation_id, request).await;
        self.dispatch_post_action(&outcome);
        Ok(outcome)
    }

    /// Accept a request, then run it on a background task.
    ///
    /// Used by the control RPC so the socket connection is not held open for the
    /// duration of a multi-minute build. Callers observe progress via
    /// `events.since(seq)`. The returned response carries the operation id.
    pub async fn submit_detached(&self, actor: Actor, request: ControlRequest) -> ControlResponse {
        let response = self.control.lock().await.accept(actor, request.clone());
        let ControlResponse::Accepted { operation_id, .. } = &response else {
            return response;
        };

        let operation_id = operation_id.clone();
        let this = self.clone();
        tokio::spawn(async move {
            let outcome = this.run_accepted(operation_id, request).await;
            this.dispatch_post_action(&outcome);
        });

        response
    }

    /// Route a dangerous control request (spec §7 dangerous ops list):
    ///
    /// - **Feishu** actors must confirm via a card. The watchdog creates a
    ///   single-use, short-lived grant and holds the pending request keyed by
    ///   the returned opaque token (`PendingConfirmation` response).
    /// - **Cli** actors are a trusted local console (the private unix socket +
    ///   startup secret): they execute directly, matching pre-Phase-3 behavior.
    ///
    /// This is the single gate for dangerous actions from the Feishu adapter.
    pub async fn submit_or_confirm(
        &self,
        actor: Actor,
        request: ControlRequest,
    ) -> RpcControlResponse {
        match &actor {
            Actor::Feishu {
                chat_id: Some(chat),
                ..
            } => match self.create_confirmation(&actor, &request, chat).await {
                Ok(created) => RpcControlResponse::PendingConfirmation {
                    token: created.token,
                    action: created.action,
                    message: created.message,
                    expires_in: created.expires_in,
                },
                Err(e) => RpcControlResponse::Rejected {
                    code: "confirmation_required".into(),
                    message: e.to_string(),
                },
            },
            Actor::Feishu { chat_id: None, .. } => RpcControlResponse::Rejected {
                code: "confirmation_required".into(),
                message: "Feishu dangerous action requires a chat channel for confirmation".into(),
            },
            _ => self.submit_detached(actor, request).await.into(),
        }
    }

    /// Create a confirmation grant for a dangerous action and hold the pending
    /// request keyed by the returned opaque token. The principal is derived by
    /// the watchdog (not trusted from the envelope), and the channel is the
    /// originating chat — both bound into the grant (spec §7/§6.3).
    async fn create_confirmation(
        &self,
        actor: &Actor,
        request: &ControlRequest,
        channel: &str,
    ) -> Result<ConfirmationCreated> {
        let principal = actor_to_principal(actor).ok_or_else(|| {
            SebasError::Upgrade("cannot derive a confirmation principal for this actor".into())
        })?;
        let (action, params) = confirmation_context(request);
        let token = self.confirmation.create_grant(
            principal,
            action.clone(),
            channel.to_string(),
            "watchdog".to_string(),
            params,
            CONFIRMATION_TTL_SECS,
        );
        self.pending.lock().await.insert(
            token.clone(),
            PendingControl {
                actor: actor.clone(),
                request: request.clone(),
            },
        );
        Ok(ConfirmationCreated {
            token,
            action,
            message: confirmation_message(request),
            expires_in: CONFIRMATION_TTL_SECS,
        })
    }

    /// Finalize a pending dangerous action by redeeming its confirmation token.
    ///
    /// The client sends only the opaque token; the canonical params come from
    /// the stored pending request, so params cannot be tampered with over the
    /// wire (spec §6.3). The grant is single-use and atomic: concurrent
    /// double-clicks yield exactly one execution.
    pub async fn confirm(
        &self,
        token: &str,
        principal: &AssertionPrincipal,
        channel: &str,
    ) -> RpcControlResponse {
        let stored = { self.pending.lock().await.get(token).cloned() };
        let Some(stored) = stored else {
            return RpcControlResponse::Rejected {
                code: "confirmation_required".into(),
                message: "unknown or already handled confirmation token".into(),
            };
        };
        let params = confirmation_params(&stored.request);
        match self.confirmation.redeem(token, principal, channel, &params) {
            Ok(_) => {
                self.pending.lock().await.remove(token);
                self.submit_detached(stored.actor, stored.request)
                    .await
                    .into()
            }
            Err(ConfirmationError::AlreadyRedeemed) => RpcControlResponse::Rejected {
                code: "already_redeemed".into(),
                message: "this confirmation was already handled".into(),
            },
            Err(ConfirmationError::Expired) => RpcControlResponse::Rejected {
                code: "confirmation_expired".into(),
                message: "confirmation token has expired".into(),
            },
            Err(_) => RpcControlResponse::Rejected {
                code: "unauthorized".into(),
                message: "confirmation token does not match this actor/channel".into(),
            },
        }
    }

    /// Cancel a pending dangerous action. Redeems (consumes) the grant so it
    /// cannot be confirmed afterwards, and records a Canceled event for the
    /// audit trail (spec §9). Like confirm, validates the caller's principal
    /// and channel against the grant.
    pub async fn cancel(
        &self,
        token: &str,
        principal: &AssertionPrincipal,
        channel: &str,
    ) -> RpcControlResponse {
        let stored = { self.pending.lock().await.get(token).cloned() };
        let Some(stored) = stored else {
            return RpcControlResponse::Rejected {
                code: "confirmation_required".into(),
                message: "unknown or already handled confirmation token".into(),
            };
        };
        let params = confirmation_params(&stored.request);
        match self.confirmation.redeem(token, principal, channel, &params) {
            Ok(_) => {
                self.pending.lock().await.remove(token);
                let op_id = format!("cfm_{token}");
                self.control
                    .lock()
                    .await
                    .record_canceled(&op_id, "confirmation canceled by user");
                RpcControlResponse::Accepted {
                    operation_id: op_id,
                    status: "Canceled".into(),
                }
            }
            Err(ConfirmationError::AlreadyRedeemed) => RpcControlResponse::Rejected {
                code: "already_redeemed".into(),
                message: "this confirmation was already handled".into(),
            },
            Err(ConfirmationError::Expired) => RpcControlResponse::Rejected {
                code: "confirmation_expired".into(),
                message: "confirmation token has expired".into(),
            },
            Err(_) => RpcControlResponse::Rejected {
                code: "unauthorized".into(),
                message: "confirmation token does not match this actor/channel".into(),
            },
        }
    }

    /// Reserve an operation slot, converting a rejection into an error.
    async fn accept(&self, actor: Actor, request: ControlRequest) -> Result<String> {
        match self.control.lock().await.accept(actor, request) {
            ControlResponse::Accepted { operation_id, .. } => Ok(operation_id),
            ControlResponse::Rejected { message, .. } => Err(SebasError::Upgrade(message)),
        }
    }

    /// Run an already-accepted operation and settle it.
    ///
    /// Every exit path settles the operation, so the exclusive lock is always
    /// released. `AssertUnwindSafe` + `catch_unwind` covers a panicking runner:
    /// without it, a panic would leave `running_exclusive` set forever.
    async fn run_accepted(
        &self,
        operation_id: String,
        request: ControlRequest,
    ) -> ExecutionOutcome {
        let execution = self.plan_for(&request);

        match execution {
            Execution::Nothing => {
                // Non-executing request (Status, service queries): nothing to run,
                // but it still occupies a record and must be settled.
                self.control
                    .lock()
                    .await
                    .mark_done(&operation_id, "no execution required");
                ExecutionOutcome {
                    operation_id,
                    post_action: PostAction::None,
                }
            }
            Execution::RestartOnly => {
                self.control
                    .lock()
                    .await
                    .mark_done(&operation_id, "restarting core");
                ExecutionOutcome {
                    operation_id,
                    post_action: PostAction::RestartCore { is_upgrade: false },
                }
            }
            Execution::Updater { plan, label } => {
                self.control
                    .lock()
                    .await
                    .mark_running(&operation_id, format!("running {label}"));
                info!(operation_id = %operation_id, "executing {label}");

                let result = {
                    use futures_util::FutureExt;
                    let fut = self.runner.run(&plan, &self.config);
                    match std::panic::AssertUnwindSafe(fut).catch_unwind().await {
                        Ok(result) => result,
                        Err(_) => Err(SebasError::Upgrade(format!("{label} panicked"))),
                    }
                };

                match result {
                    Err(error) => {
                        warn!(operation_id = %operation_id, "{label} failed: {error}");
                        self.control
                            .lock()
                            .await
                            .mark_error(&operation_id, format!("{label} failed: {error}"));
                        ExecutionOutcome {
                            operation_id,
                            post_action: PostAction::None,
                        }
                    }
                    Ok(()) => {
                        let post_action = if plan.dry_run {
                            PostAction::None
                        } else {
                            PostAction::RestartCore { is_upgrade: true }
                        };
                        let message = if plan.dry_run {
                            format!("{label} dry-run completed")
                        } else {
                            format!("{label} completed; restarting core")
                        };
                        self.control.lock().await.mark_done(&operation_id, message);
                        ExecutionOutcome {
                            operation_id,
                            post_action,
                        }
                    }
                }
            }
        }
    }

    /// Map a control request onto an execution path.
    fn plan_for(&self, request: &ControlRequest) -> Execution {
        match request {
            ControlRequest::Update {
                kind,
                dry_run,
                target,
            } => {
                let dev = matches!(kind, UpdateKind::Dev);
                let project_dir = match target {
                    Some(UpdateTarget::ProjectDir(dir)) => Some(dir.clone()),
                    Some(UpdateTarget::ConfiguredDevTarget { .. }) | None => None,
                };
                Execution::Updater {
                    plan: UpdatePlan {
                        config_path: self.config_path.clone(),
                        dev,
                        dry_run: *dry_run,
                        rollback: false,
                        project_dir,
                    },
                    label: if dev { "dev update" } else { "release update" },
                }
            }
            ControlRequest::Rollback { dry_run } => Execution::Updater {
                plan: UpdatePlan {
                    config_path: self.config_path.clone(),
                    dev: false,
                    dry_run: *dry_run,
                    rollback: true,
                    project_dir: None,
                },
                label: "rollback",
            },
            ControlRequest::RestartCore => {
                // Restart needs no installer work — settle immediately and
                // signal the supervisor to restart the core child.
                Execution::RestartOnly
            }
            _ => Execution::Nothing,
        }
    }

    fn dispatch_post_action(&self, outcome: &ExecutionOutcome) {
        if outcome.post_action == PostAction::None {
            return;
        }
        // A closed channel means the supervisor loop is gone (shutting down);
        // there is nothing useful to do beyond noting it.
        if self.restart_tx.send(outcome.post_action).is_err() {
            warn!("core restart requested but supervisor channel is closed");
        }
    }

    /// Return the current status of all managed services.
    pub async fn service_status(&self) -> RpcControlResponse {
        use crate::watchdog::control::OperationStatus;
        let control = self.control.lock().await;

        // Check if there's a running exclusive operation (update/rollback/restart).
        let updater_status = match &control.running_exclusive() {
            Some(op_id) => {
                if let Some(record) = control.operation(op_id) {
                    match record.status {
                        OperationStatus::Running => "running",
                        _ => "pending",
                    }
                } else {
                    "idle"
                }
            }
            None => "idle",
        };

        // Check if the core has been accepted recently (simple heuristic:
        // if there are operations, the core is reachable).
        let core_status = if control.operation_count() > 0 {
            "running"
        } else {
            "unknown"
        };

        let services = vec![
            RpcServiceStatus {
                name: "watchdog".into(),
                status: "running".into(),
                desired: "enabled".into(),
                uptime_secs: None,
            },
            RpcServiceStatus {
                name: "core".into(),
                status: core_status.into(),
                desired: "enabled".into(),
                uptime_secs: None,
            },
            RpcServiceStatus {
                name: "updater".into(),
                status: updater_status.into(),
                desired: "enabled".into(),
                uptime_secs: None,
            },
            RpcServiceStatus {
                name: "webui".into(),
                status: "running".into(),
                desired: "enabled".into(),
                uptime_secs: None,
            },
            RpcServiceStatus {
                name: "gateway".into(),
                status: "running".into(),
                desired: "enabled".into(),
                uptime_secs: None,
            },
            RpcServiceStatus {
                name: "feishu".into(),
                status: "running".into(),
                desired: "enabled".into(),
                uptime_secs: None,
            },
        ];

        RpcControlResponse::Services { services }
    }

    /// Return the current status of a single managed service, or an empty
    /// service list when the service is unknown (used by `/gateway status`
    /// and `/webui status`, spec §12).
    pub async fn service_status_for(&self, service: &str) -> RpcControlResponse {
        match self.service_status().await {
            RpcControlResponse::Services { services } => RpcControlResponse::Services {
                services: services.into_iter().filter(|s| s.name == service).collect(),
            },
            other => other,
        }
    }
}

/// Human-readable action id + canonical normalized params for a dangerous
/// control request (spec §7 dangerous ops list). The same context is used at
/// grant creation and at redemption, so the wire only ever carries the opaque
/// token — there is nothing for a client to tamper with.
fn confirmation_context(request: &ControlRequest) -> (String, HashMap<String, String>) {
    let mut params = HashMap::new();
    let action = match request {
        ControlRequest::Update {
            kind,
            dry_run,
            target,
        } => {
            params.insert("dry_run".to_string(), dry_run.to_string());
            if let Some(UpdateTarget::ProjectDir(dir)) = target {
                params.insert("target".to_string(), dir.display().to_string());
            }
            match kind {
                UpdateKind::Release => "update_release",
                UpdateKind::Dev => "update_dev",
            }
            .to_string()
        }
        ControlRequest::Rollback { dry_run } => {
            params.insert("dry_run".to_string(), dry_run.to_string());
            "rollback".to_string()
        }
        ControlRequest::RestartCore => "restart_core".to_string(),
        other => format!("{other:?}").to_lowercase(),
    };
    (action, params)
}

/// Canonical params for a request — used both at grant creation and at
/// redemption. Since confirm/cancel only receive the opaque token, params are
/// always the stored (canonical) ones.
fn confirmation_params(request: &ControlRequest) -> HashMap<String, String> {
    let (_, params) = confirmation_context(request);
    params
}

/// User-facing message shown on the confirmation card.
fn confirmation_message(request: &ControlRequest) -> String {
    match request {
        ControlRequest::Update {
            kind: UpdateKind::Release,
            ..
        } => "将更新到最新发布版本（release）。".to_string(),
        ControlRequest::Update {
            kind: UpdateKind::Dev,
            ..
        } => "将更新到开发版本（dev）。".to_string(),
        ControlRequest::Rollback { .. } => "将回滚到上一个可用版本。".to_string(),
        ControlRequest::RestartCore => "将重启 core 进程（短暂中断）。".to_string(),
        other => format!("将执行 {other:?}。"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watchdog::control::{ErrorCode, OperationStatus};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Records how many times it ran and what it was asked to do.
    #[derive(Default)]
    struct FakeRunner {
        calls: AtomicUsize,
        fail: bool,
        panic: bool,
        seen_dev: std::sync::Mutex<Vec<bool>>,
    }

    #[async_trait::async_trait]
    impl UpdaterRunner for FakeRunner {
        async fn run(&self, plan: &UpdatePlan, _watchdog: &WatchdogConfig) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen_dev.lock().unwrap().push(plan.dev);
            if self.panic {
                panic!("runner exploded");
            }
            if self.fail {
                return Err(SebasError::Upgrade("fake failure".into()));
            }
            Ok(())
        }
    }

    fn executor_with(
        runner: Arc<dyn UpdaterRunner>,
    ) -> (
        ControlExecutor,
        Arc<Mutex<ControlService>>,
        mpsc::UnboundedReceiver<PostAction>,
    ) {
        let control = Arc::new(Mutex::new(ControlService::new()));
        let (tx, rx) = mpsc::unbounded_channel();
        let executor = ControlExecutor::new(
            control.clone(),
            runner,
            WatchdogConfig::default(),
            "./config.toml".into(),
            tx,
        );
        (executor, control, rx)
    }

    fn release_update(dry_run: bool) -> ControlRequest {
        ControlRequest::Update {
            kind: UpdateKind::Release,
            dry_run,
            target: None,
        }
    }

    #[tokio::test]
    async fn successful_update_runs_runner_and_requests_restart() {
        let runner = Arc::new(FakeRunner::default());
        let (executor, control, mut rx) = executor_with(runner.clone());

        let outcome = executor
            .submit_blocking(Actor::System, release_update(false))
            .await
            .expect("update must be accepted");

        assert_eq!(
            runner.calls.load(Ordering::SeqCst),
            1,
            "runner must execute"
        );
        assert_eq!(
            outcome.post_action,
            PostAction::RestartCore { is_upgrade: true }
        );
        assert_eq!(
            rx.try_recv().ok(),
            Some(PostAction::RestartCore { is_upgrade: true })
        );

        let control = control.lock().await;
        let op = control.operation(&outcome.operation_id).expect("record");
        assert_eq!(op.status, OperationStatus::Succeeded);
    }

    #[tokio::test]
    async fn dry_run_does_not_request_restart() {
        let runner = Arc::new(FakeRunner::default());
        let (executor, _control, mut rx) = executor_with(runner.clone());

        let outcome = executor
            .submit_blocking(Actor::System, release_update(true))
            .await
            .expect("dry-run must be accepted");

        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(outcome.post_action, PostAction::None);
        assert!(rx.try_recv().is_err(), "dry-run must not restart core");
    }

    #[tokio::test]
    async fn failed_update_settles_operation_and_skips_restart() {
        let runner = Arc::new(FakeRunner {
            fail: true,
            ..Default::default()
        });
        let (executor, control, mut rx) = executor_with(runner.clone());

        let outcome = executor
            .submit_blocking(Actor::System, release_update(false))
            .await
            .expect("accept succeeds even though the run fails");

        assert_eq!(outcome.post_action, PostAction::None);
        assert!(
            rx.try_recv().is_err(),
            "failed update must not restart core"
        );

        let control = control.lock().await;
        let op = control.operation(&outcome.operation_id).expect("record");
        assert_eq!(op.status, OperationStatus::Failed);
    }

    /// The regression test for the deadlock: a first operation must never leave
    /// the exclusive lock held.
    #[tokio::test]
    async fn consecutive_updates_do_not_deadlock_on_the_exclusive_lock() {
        let runner = Arc::new(FakeRunner::default());
        let (executor, _control, _rx) = executor_with(runner.clone());

        for attempt in 1..=3 {
            executor
                .submit_blocking(Actor::System, release_update(false))
                .await
                .unwrap_or_else(|e| panic!("update #{attempt} must not be rejected: {e}"));
        }

        assert_eq!(
            runner.calls.load(Ordering::SeqCst),
            3,
            "all three updates must actually run"
        );
    }

    #[tokio::test]
    async fn failed_update_releases_lock_for_the_next_one() {
        let (executor, _control, _rx) = executor_with(Arc::new(FakeRunner {
            fail: true,
            ..Default::default()
        }));

        // First fails...
        let first = executor
            .submit_blocking(Actor::System, release_update(false))
            .await
            .expect("accepted");
        assert_eq!(first.post_action, PostAction::None);

        // ...and must not wedge the second.
        executor
            .submit_blocking(Actor::System, release_update(false))
            .await
            .expect("second update must not be Busy after a failure");
    }

    #[tokio::test]
    async fn panicking_runner_still_releases_the_lock() {
        let (executor, control, _rx) = executor_with(Arc::new(FakeRunner {
            panic: true,
            ..Default::default()
        }));

        let outcome = executor
            .submit_blocking(Actor::System, release_update(false))
            .await
            .expect("accepted");

        {
            let control = control.lock().await;
            let op = control.operation(&outcome.operation_id).expect("record");
            assert_eq!(
                op.status,
                OperationStatus::Failed,
                "a panicking runner must mark the operation failed"
            );
        }

        // The lock must be free again.
        executor
            .submit_blocking(Actor::System, release_update(false))
            .await
            .expect("panic must not wedge the exclusive lock");
    }

    #[tokio::test]
    async fn detached_submit_returns_immediately_then_settles() {
        let runner = Arc::new(FakeRunner::default());
        let (executor, control, mut rx) = executor_with(runner.clone());

        let response = executor
            .submit_detached(Actor::Cli { uid: 1000 }, release_update(false))
            .await;
        let ControlResponse::Accepted { operation_id, .. } = response else {
            panic!("detached submit must be accepted");
        };

        // Background task settles it; wait for the restart signal.
        let action = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("background execution must finish")
            .expect("channel open");
        assert_eq!(action, PostAction::RestartCore { is_upgrade: true });

        let control = control.lock().await;
        let op = control.operation(&operation_id).expect("record");
        assert_eq!(op.status, OperationStatus::Succeeded);
    }

    #[tokio::test]
    async fn detached_submit_releases_lock_so_rpc_callers_are_not_wedged() {
        let runner = Arc::new(FakeRunner::default());
        let (executor, _control, mut rx) = executor_with(runner.clone());

        for _ in 0..2 {
            let response = executor
                .submit_detached(Actor::Cli { uid: 1000 }, release_update(false))
                .await;
            assert!(
                matches!(response, ControlResponse::Accepted { .. }),
                "detached RPC update must not be rejected as Busy"
            );
            // Let the background task settle before the next submit.
            tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("execution finishes")
                .expect("channel open");
        }

        assert_eq!(runner.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn dev_update_reaches_the_runner_as_a_dev_plan() {
        let runner = Arc::new(FakeRunner::default());
        let (executor, _control, _rx) = executor_with(runner.clone());

        executor
            .submit_blocking(
                Actor::System,
                ControlRequest::Update {
                    kind: UpdateKind::Dev,
                    dry_run: false,
                    target: None,
                },
            )
            .await
            .expect("dev update accepted");

        assert_eq!(runner.seen_dev.lock().unwrap().as_slice(), &[true]);
    }

    #[tokio::test]
    async fn rollback_reaches_the_runner_as_a_rollback_plan() {
        #[derive(Default)]
        struct RollbackSpy {
            saw_rollback: std::sync::Mutex<Vec<bool>>,
        }

        #[async_trait::async_trait]
        impl UpdaterRunner for RollbackSpy {
            async fn run(&self, plan: &UpdatePlan, _w: &WatchdogConfig) -> Result<()> {
                self.saw_rollback.lock().unwrap().push(plan.rollback);
                Ok(())
            }
        }

        let runner = Arc::new(RollbackSpy::default());
        let (executor, _control, _rx) = executor_with(runner.clone());

        executor
            .submit_blocking(Actor::System, ControlRequest::Rollback { dry_run: false })
            .await
            .expect("rollback accepted");

        assert_eq!(runner.saw_rollback.lock().unwrap().as_slice(), &[true]);
    }

    /// Concurrency check: while one exclusive op is genuinely in flight, a second
    /// must be rejected Busy — the lock still has to *work*, not just release.
    #[tokio::test]
    async fn concurrent_exclusive_operation_is_rejected_while_running() {
        struct Blocking {
            gate: tokio::sync::Notify,
        }

        #[async_trait::async_trait]
        impl UpdaterRunner for Blocking {
            async fn run(&self, _plan: &UpdatePlan, _w: &WatchdogConfig) -> Result<()> {
                self.gate.notified().await;
                Ok(())
            }
        }

        let runner = Arc::new(Blocking {
            gate: tokio::sync::Notify::new(),
        });
        let (executor, _control, _rx) = executor_with(runner.clone());

        // Start one and leave it parked inside the runner.
        let first = executor
            .submit_detached(Actor::System, release_update(false))
            .await;
        assert!(matches!(first, ControlResponse::Accepted { .. }));

        // Give the spawned task a chance to reach the runner.
        tokio::task::yield_now().await;

        let second = executor
            .submit_detached(Actor::System, release_update(false))
            .await;
        assert!(
            matches!(
                second,
                ControlResponse::Rejected {
                    code: ErrorCode::Busy,
                    ..
                }
            ),
            "a second exclusive op must be Busy while the first is running, got {second:?}"
        );

        runner.gate.notify_waiters();
    }
}
