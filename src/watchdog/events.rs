use std::collections::VecDeque;
use std::fmt;
use std::time::SystemTime;

const DEFAULT_TIMELINE_CAPACITY: usize = 200;

/// A diagnostic string that redacts known secret patterns on [`Display`].
///
/// Use this when constructing public-facing log or event messages that
/// may contain sensitive values (tokens, passwords, API keys).
///
/// This is a baseline implementation — it replaces known `key=value`
/// patterns with `***` but is **not** a comprehensive sanitizer.
#[derive(Debug, Clone)]
pub struct RedactedDiagnostic(String);

impl RedactedDiagnostic {
    /// Wrap a raw string (possibly containing secrets).
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Consume the wrapper and return the original (unredacted) string.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for RedactedDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        static KEYS: &[&str] = &[
            "secret",
            "token",
            "password",
            "api_key",
            "authorization",
        ];
        let s = redact_keys(&self.0, KEYS);
        write!(f, "{s}")
    }
}

/// Replace every occurrence of `key=<value>` with `key=***` for each key in
/// `keys`. The value is everything until the next whitespace, comma, closing
/// bracket, quote, pipe, or end of string.
fn redact_keys(s: &str, keys: &[&str]) -> String {
    let mut result = s.to_string();
    for key in keys {
        let pattern = format!("{key}=");
        let mut search_start = 0;
        while let Some(pos) = result[search_start..].find(&pattern) {
            let abs_pos = search_start + pos;
            let value_start = abs_pos + pattern.len();
            let tail = &result[value_start..];
            let value_end = tail
                .find(|c: char| {
                    c.is_whitespace()
                        || c == ','
                        || c == '}'
                        || c == ']'
                        || c == '"'
                        || c == '\''
                        || c == ')'
                        || c == '|'
                })
                .unwrap_or(tail.len());
            let replacement = format!("{key}=***");
            result.replace_range(abs_pos..value_start + value_end, &replacement);
            search_start = abs_pos + replacement.len();
        }
    }
    result
}

/// Status of a control-plane operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationStatus {
    PendingConfirmation,
    Accepted,
    Running,
    Succeeded,
    Failed,
    Canceled,
    TimedOut,
}

/// Error codes for rejected control operations.
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

/// Kind of event recorded in the control-plane timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEventKind {
    Started,
    Progress,
    Done,
    Error,
    /// The operation was canceled by a user or external signal.
    Canceled,
    /// The operation exceeded its deadline.
    TimedOut,
}

/// A single event in the control-plane timeline.
#[derive(Debug, Clone)]
pub struct ControlEvent {
    pub seq: u64,
    pub timestamp: SystemTime,
    pub operation_id: String,
    pub kind: ControlEventKind,
    pub public_message: String,
}

/// Bounded in-memory timeline of control-plane events.
///
/// Events are stored in a [`VecDeque`] with a fixed capacity. When the
/// capacity is reached, the oldest event is evicted. This is suitable for
/// short-lived diagnostic / polling use but is **not** a durable audit log.
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
    /// Create a new timeline with the given capacity (minimum 1).
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_seq: 1,
            events: VecDeque::new(),
        }
    }

    /// Push a new event onto the timeline, returning the event.
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

    /// Return all events with a sequence number greater than `seq`.
    pub fn since(&self, seq: u64) -> Vec<ControlEvent> {
        self.events
            .iter()
            .filter(|event| event.seq > seq)
            .cloned()
            .collect()
    }

    /// Return all events currently in the timeline.
    pub fn all(&self) -> Vec<ControlEvent> {
        self.events.iter().cloned().collect()
    }
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
    fn audit_redaction() {
        let diag = RedactedDiagnostic::new("secret=my-secret-value");
        let displayed = format!("{diag}");
        assert!(!displayed.contains("my-secret-value"));
        assert!(displayed.contains("secret=***"));

        let diag = RedactedDiagnostic::new("token=abc123");
        let displayed = format!("{diag}");
        assert!(!displayed.contains("abc123"));
        assert!(displayed.contains("token=***"));

        let diag = RedactedDiagnostic::new("password=s3cret!");
        let displayed = format!("{diag}");
        assert!(!displayed.contains("s3cret!"));
        assert!(displayed.contains("password=***"));

        // non-secret text should pass through unchanged
        let diag = RedactedDiagnostic::new("hello world");
        assert_eq!(format!("{diag}"), "hello world");
    }

    #[test]
    fn duplicate_idempotency_returns_same_operation() {
        use crate::watchdog::control::{
            Actor, ControlRequest, ControlResponse, ControlService, UpdateKind,
        };

        let mut svc = ControlService::new();
        let first = svc.accept_idempotent(
            "key-1",
            Actor::System,
            ControlRequest::Update {
                kind: UpdateKind::Release,
                dry_run: false,
                target: None,
            },
        );
        let second = svc.accept_idempotent(
            "key-1",
            Actor::System,
            ControlRequest::Update {
                kind: UpdateKind::Release,
                dry_run: false,
                target: None,
            },
        );

        match (first, second) {
            (
                ControlResponse::Accepted {
                    operation_id: a, ..
                },
                ControlResponse::Accepted {
                    operation_id: b, ..
                },
            ) => {
                assert_eq!(
                    a, b,
                    "same idempotency key must return the same operation_id"
                );
            }
            _ => panic!("both calls must be Accepted"),
        }
    }

    #[test]
    fn cancel_operation() {
        use crate::watchdog::control::{
            Actor, ControlRequest, ControlResponse, ControlService, UpdateKind,
        };

        let mut svc = ControlService::new();
        let resp = svc.accept(
            Actor::System,
            ControlRequest::Update {
                kind: UpdateKind::Release,
                dry_run: false,
                target: None,
            },
        );
        let ControlResponse::Accepted { operation_id, .. } = resp else {
            panic!("expected Accepted");
        };

        svc.mark_canceled(&operation_id, "canceled by user");
        let record = svc.operation(&operation_id).unwrap();
        assert_eq!(record.status, OperationStatus::Canceled);

        let events = svc.events_since(0);
        assert!(events.iter().any(|e| e.kind == ControlEventKind::Canceled));
    }

    #[test]
    fn timeout_operation() {
        use crate::watchdog::control::{
            Actor, ControlRequest, ControlResponse, ControlService, UpdateKind,
        };

        let mut svc = ControlService::new();
        let resp = svc.accept(
            Actor::System,
            ControlRequest::Update {
                kind: UpdateKind::Release,
                dry_run: false,
                target: None,
            },
        );
        let ControlResponse::Accepted { operation_id, .. } = resp else {
            panic!("expected Accepted");
        };

        svc.mark_timed_out(&operation_id, "operation timed out");
        let record = svc.operation(&operation_id).unwrap();
        assert_eq!(record.status, OperationStatus::TimedOut);

        let events = svc.events_since(0);
        assert!(events.iter().any(|e| e.kind == ControlEventKind::TimedOut));
    }
}