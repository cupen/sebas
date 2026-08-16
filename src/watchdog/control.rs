use std::collections::HashMap;
use std::path::PathBuf;

// Re-export event types extracted to events.rs for backward compatibility.
pub use crate::watchdog::events::{
    ControlEvent, ControlEventKind, ControlEventTimeline, ErrorCode, OperationStatus,
    RedactedDiagnostic,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlRequest {
    Status,
    RestartCore,
    StopCore,
    StartCore,
    Update {
        kind: UpdateKind,
        dry_run: bool,
        target: Option<UpdateTarget>,
    },
    Rollback {
        dry_run: bool,
    },
    ServiceSet {
        service: ManagedService,
        desired: DesiredState,
        persist: bool,
    },
    ServiceRestart {
        service: ManagedService,
    },
    ServiceStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateKind {
    Release,
    Dev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateTarget {
    /// Temporary bridge for the existing `sebas update --dev --project-dir` path.
    /// Phase 1/3 should replace remote-facing callers with configured target names.
    ProjectDir(PathBuf),
    ConfiguredDevTarget {
        name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManagedService {
    WebUi,
    Gateway,
    Feishu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesiredState {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    WebUi {
        user: Option<String>,
        local: bool,
    },
    Feishu {
        open_id: String,
        chat_id: Option<String>,
    },
    Cli {
        uid: u32,
    },
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlResponse {
    Accepted {
        operation_id: String,
        status: OperationStatus,
    },
    Rejected {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct OperationRecord {
    pub operation_id: String,
    pub actor: Actor,
    pub request: ControlRequest,
    pub status: OperationStatus,
}

#[derive(Debug)]
pub struct ControlService {
    timeline: ControlEventTimeline,
    operations: HashMap<String, OperationRecord>,
    running_exclusive: Option<String>,
    next_operation: u64,
    idempotent_ops: HashMap<String, String>,
}

impl ControlService {
    pub fn new() -> Self {
        Self {
            timeline: ControlEventTimeline::default(),
            operations: HashMap::new(),
            running_exclusive: None,
            next_operation: 1,
            idempotent_ops: HashMap::new(),
        }
    }

    pub fn accept_idempotent(
        &mut self,
        idempotency_key: &str,
        actor: Actor,
        request: ControlRequest,
    ) -> ControlResponse {
        if let Some(op_id) = self.idempotent_ops.get(idempotency_key) {
            if let Some(record) = self.operations.get(op_id) {
                return ControlResponse::Accepted {
                    operation_id: op_id.clone(),
                    status: record.status,
                };
            }
        }
        let response = self.accept(actor, request);
        if let ControlResponse::Accepted { ref operation_id, .. } = response {
            self.idempotent_ops
                .insert(idempotency_key.to_string(), operation_id.clone());
        }
        response
    }

    pub fn accept(&mut self, actor: Actor, request: ControlRequest) -> ControlResponse {
        if is_exclusive(&request) {
            if self.running_exclusive.is_some() {
                return ControlResponse::Rejected {
                    code: ErrorCode::Busy,
                    message: "another exclusive control operation is running".into(),
                };
            }
        }

        let operation_id = self.next_operation_id();
        let record = OperationRecord {
            operation_id: operation_id.clone(),
            actor,
            request: request.clone(),
            status: OperationStatus::Accepted,
        };
        if is_exclusive(&request) {
            self.running_exclusive = Some(operation_id.clone());
        }
        self.operations.insert(operation_id.clone(), record);
        self.timeline.push(
            operation_id.clone(),
            ControlEventKind::Started,
            format!("accepted {:?}", request),
        );
        ControlResponse::Accepted {
            operation_id,
            status: OperationStatus::Accepted,
        }
    }

    pub fn mark_running(&mut self, operation_id: &str, message: impl Into<String>) {
        self.set_status(operation_id, OperationStatus::Running);
        self.timeline
            .push(operation_id, ControlEventKind::Progress, message);
    }

    pub fn mark_done(&mut self, operation_id: &str, message: impl Into<String>) {
        self.set_status(operation_id, OperationStatus::Succeeded);
        self.finish_exclusive(operation_id);
        self.timeline
            .push(operation_id, ControlEventKind::Done, message);
    }

    pub fn mark_error(&mut self, operation_id: &str, message: impl Into<String>) {
        self.set_status(operation_id, OperationStatus::Failed);
        self.finish_exclusive(operation_id);
        self.timeline
            .push(operation_id, ControlEventKind::Error, message);
    }

    /// Mark an operation as canceled by a user or external signal.
    pub fn mark_canceled(&mut self, operation_id: &str, message: impl Into<String>) {
        self.set_status(operation_id, OperationStatus::Canceled);
        self.finish_exclusive(operation_id);
        self.timeline
            .push(operation_id, ControlEventKind::Canceled, message);
    }

    /// Mark an operation as timed out (exceeded its deadline).
    pub fn mark_timed_out(&mut self, operation_id: &str, message: impl Into<String>) {
        self.set_status(operation_id, OperationStatus::TimedOut);
        self.finish_exclusive(operation_id);
        self.timeline
            .push(operation_id, ControlEventKind::TimedOut, message);
    }

    /// Record a Canceled event for an operation that never reached `Accepted`
    /// (e.g. a pending confirmation canceled by the user, Phase 3 Task 3.2).
    /// Pushes to the timeline only — there is no operation record to update.
    pub fn record_canceled(&mut self, operation_id: &str, message: impl Into<String>) {
        self.timeline
            .push(operation_id, ControlEventKind::Canceled, message);
    }

    pub fn events_since(&self, seq: u64) -> Vec<ControlEvent> {
        self.timeline.since(seq)
    }

    pub fn operation(&self, operation_id: &str) -> Option<&OperationRecord> {
        self.operations.get(operation_id)
    }

    pub fn running_exclusive(&self) -> Option<&str> {
        self.running_exclusive.as_deref()
    }

    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    fn next_operation_id(&mut self) -> String {
        let id = format!("op_{}", self.next_operation);
        self.next_operation += 1;
        id
    }

    fn set_status(&mut self, operation_id: &str, status: OperationStatus) {
        if let Some(op) = self.operations.get_mut(operation_id) {
            op.status = status;
        }
    }

    fn finish_exclusive(&mut self, operation_id: &str) {
        if self.running_exclusive.as_deref() == Some(operation_id) {
            self.running_exclusive = None;
        }
    }
}

impl Default for ControlService {
    fn default() -> Self {
        Self::new()
    }
}

fn is_exclusive(request: &ControlRequest) -> bool {
    matches!(
        request,
        ControlRequest::RestartCore
            | ControlRequest::StopCore
            | ControlRequest::StartCore
            | ControlRequest::Update { .. }
            | ControlRequest::Rollback { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_operations_return_busy_until_finished() {
        let mut control = ControlService::new();
        let first = control.accept(
            Actor::System,
            ControlRequest::Update {
                kind: UpdateKind::Release,
                dry_run: false,
                target: None,
            },
        );
        let ControlResponse::Accepted { operation_id, .. } = first else {
            panic!("first operation must be accepted");
        };

        let second = control.accept(Actor::System, ControlRequest::RestartCore);
        assert!(matches!(
            second,
            ControlResponse::Rejected {
                code: ErrorCode::Busy,
                ..
            }
        ));

        control.mark_done(&operation_id, "done");
        let third = control.accept(Actor::System, ControlRequest::RestartCore);
        assert!(matches!(third, ControlResponse::Accepted { .. }));
    }
}
