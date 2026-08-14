use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::SystemTime;

const DEFAULT_TIMELINE_CAPACITY: usize = 200;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    PendingConfirmation,
    Accepted,
    Running,
    Succeeded,
    Failed,
    Canceled,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Busy,
    Unauthorized,
    ConfirmationRequired,
    ConfirmationExpired,
    InvalidTarget,
    Timeout,
    UpdaterFailed,
    ServiceUnavailable,
    Internal,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEventKind {
    Started,
    Progress,
    Done,
    Error,
}

#[derive(Debug, Clone)]
pub struct ControlEvent {
    pub seq: u64,
    pub timestamp: SystemTime,
    pub operation_id: String,
    pub kind: ControlEventKind,
    pub public_message: String,
}

#[derive(Debug)]
pub struct ControlEventTimeline {
    capacity: usize,
    next_seq: u64,
    events: VecDeque<ControlEvent>,
}

impl Default for ControlEventTimeline {
    fn default() -> Self {
        Self::new(DEFAULT_TIMELINE_CAPACITY)
    }
}

impl ControlEventTimeline {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_seq: 1,
            events: VecDeque::new(),
        }
    }

    pub fn push(
        &mut self,
        operation_id: impl Into<String>,
        kind: ControlEventKind,
        public_message: impl Into<String>,
    ) -> ControlEvent {
        let event = ControlEvent {
            seq: self.next_seq,
            timestamp: SystemTime::now(),
            operation_id: operation_id.into(),
            kind,
            public_message: public_message.into(),
        };
        self.next_seq += 1;
        self.events.push_back(event.clone());
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
        event
    }

    pub fn since(&self, seq: u64) -> Vec<ControlEvent> {
        self.events
            .iter()
            .filter(|event| event.seq > seq)
            .cloned()
            .collect()
    }

    pub fn all(&self) -> Vec<ControlEvent> {
        self.events.iter().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub struct OperationRecord {
    pub operation_id: String,
    pub actor: Actor,
    pub request: ControlRequest,
    pub status: OperationStatus,
}

#[derive(Debug, Default)]
pub struct ControlService {
    timeline: ControlEventTimeline,
    operations: HashMap<String, OperationRecord>,
    running_exclusive: Option<String>,
    next_operation: u64,
}

impl ControlService {
    pub fn new() -> Self {
        Self {
            timeline: ControlEventTimeline::default(),
            operations: HashMap::new(),
            running_exclusive: None,
            next_operation: 1,
        }
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

    pub fn events_since(&self, seq: u64) -> Vec<ControlEvent> {
        self.timeline.since(seq)
    }

    pub fn operation(&self, operation_id: &str) -> Option<&OperationRecord> {
        self.operations.get(operation_id)
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
    fn timeline_evicts_old_events() {
        let mut timeline = ControlEventTimeline::new(2);
        timeline.push("op_1", ControlEventKind::Progress, "one");
        timeline.push("op_1", ControlEventKind::Progress, "two");
        timeline.push("op_1", ControlEventKind::Done, "three");

        let events = timeline.all();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].public_message, "two");
        assert_eq!(events[1].public_message, "three");
    }

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
