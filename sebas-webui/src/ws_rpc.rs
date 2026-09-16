//! WS RPC protocol layer for `/ws` (add-ws-rpc-protocol).
//!
//! Three-frame envelope shared by every application-level frame on the
//! socket: [`RequestFrame`]`{id, method, params}` (client → server, id
//! unique per connection), [`ResponseFrame`]`{id, result | error}`
//! (server → client, id echoes the request; result and error are mutually
//! exclusive), [`NotificationFrame`]`{method, params}` (server → client
//! one-way push). JSON fields are snake_case and frames carry no other
//! protocol-level fields; decode tolerates unknown fields so peers may add
//! them without breaking us (`deny_unknown_fields` is deliberately not
//! set — the forward-compatibility contract).
//!
//! The wire format lives behind the [`WsCodec`] seam (design D2): JSON is
//! the first implementation, and swapping codecs changes only the byte
//! representation — never the frame semantics, id correlation, or dispatch
//! behavior.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Client → server request. `id` is unique within the connection; `params`
/// defaults to null when absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestFrame {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Semantic error carried by a [`ResponseFrame`]. `code` is a stable machine
/// string (`unknown_method`, ...); `message` faces the operator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

/// Server → client reply. `result` and `error` are mutually exclusive —
/// guaranteed by construction via [`ResponseFrame::ok`] /
/// [`ResponseFrame::error`]; the absent side is skipped on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseFrame {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl ResponseFrame {
    /// Success reply: carries `result`, never `error`.
    pub fn ok(id: u64, result: Value) -> Self {
        ResponseFrame {
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Failure reply: carries `error` (code + message), never `result`.
    pub fn error(id: u64, code: impl Into<String>, message: impl Into<String>) -> Self {
        ResponseFrame {
            id,
            result: None,
            error: Some(RpcError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

/// Server → client one-way push. The migrated event vocabulary lives here:
/// `method` = the legacy dotted type (`session.created`, `turn.append`,
/// ...), `params` = the legacy payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationFrame {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Any application-level frame on the socket. Untagged on purpose: the
/// three shapes are told apart by their fields (`id`+`method` vs
/// `id`+`result`/`error` vs bare `method`), so the wire carries no extra
/// discriminator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Frame {
    Request(RequestFrame),
    Response(ResponseFrame),
    Notification(NotificationFrame),
}

/// Decode failure: malformed JSON or a payload matching none of the three
/// frame shapes. Carries no semantics beyond "ignore this frame" — the
/// connection stays up (spec: 未知容忍).
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

/// Codec seam (design D2): the wire-format boundary for protocol frames.
/// Text frames only — the protocol does not use binary. Swapping the
/// implementation changes the byte representation, never the frame
/// semantics, id correlation, or dispatch behavior.
pub trait WsCodec: Send + Sync {
    /// Serialize a frame into one WebSocket text payload.
    fn encode(&self, frame: &Frame) -> String;
    /// Parse a text payload; `Err` for malformed JSON or a non-frame shape.
    fn decode(&self, raw: &str) -> Result<Frame, DecodeError>;
}

/// JSON first implementation (serde_json).
pub struct JsonCodec;

impl WsCodec for JsonCodec {
    fn encode(&self, frame: &Frame) -> String {
        // Infallible in practice: every frame field is a string-keyed JSON
        // member (same tolerance as the former bare-frame encoding).
        serde_json::to_string(frame).unwrap_or_default()
    }

    fn decode(&self, raw: &str) -> Result<Frame, DecodeError> {
        serde_json::from_str(raw).map_err(|e| DecodeError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CODEC: JsonCodec = JsonCodec;

    #[test]
    fn request_roundtrips_losslessly() {
        let frame = Frame::Request(RequestFrame {
            id: 7,
            method: "ping".into(),
            params: json!({"deep": [1, 2, 3]}),
        });
        let text = CODEC.encode(&frame);
        assert_eq!(CODEC.decode(&text).unwrap(), frame, "wire: {text}");
        // Missing params defaults to null (serde default), unknown fields
        // are tolerated (forward compatibility, no deny_unknown_fields).
        let got = CODEC
            .decode(r#"{"id":1,"method":"ping","future_field":true}"#)
            .unwrap();
        assert_eq!(
            got,
            Frame::Request(RequestFrame {
                id: 1,
                method: "ping".into(),
                params: Value::Null,
            })
        );
    }

    #[test]
    fn response_roundtrips_and_result_error_are_mutually_exclusive_on_the_wire() {
        let ok = Frame::Response(ResponseFrame::ok(7, json!("pong")));
        let text = CODEC.encode(&ok);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(v.get("result").is_some());
        assert!(
            v.get("error").is_none(),
            "ok reply must not carry error: {v}"
        );
        assert_eq!(CODEC.decode(&text).unwrap(), ok);

        let err = Frame::Response(ResponseFrame::error(7, "unknown_method", "no such method"));
        let text = CODEC.encode(&err);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["error"]["code"], "unknown_method");
        assert_eq!(v["error"]["message"], "no such method");
        assert!(
            v.get("result").is_none(),
            "error reply must not carry result: {v}"
        );
        assert_eq!(CODEC.decode(&text).unwrap(), err);
    }

    #[test]
    fn notification_roundtrips_losslessly() {
        let frame = Frame::Notification(NotificationFrame {
            method: "turn.append".into(),
            params: json!({"session_id": "oc_a", "seq": 4}),
        });
        let text = CODEC.encode(&frame);
        assert_eq!(CODEC.decode(&text).unwrap(), frame, "wire: {text}");
    }

    #[test]
    fn shapes_are_told_apart_without_a_wire_discriminator() {
        assert!(matches!(
            CODEC.decode(r#"{"id":2,"method":"ping"}"#).unwrap(),
            Frame::Request(_)
        ));
        assert!(matches!(
            CODEC.decode(r#"{"id":2,"result":"pong"}"#).unwrap(),
            Frame::Response(_)
        ));
        assert!(matches!(
            CODEC
                .decode(r#"{"id":2,"error":{"code":"x","message":"y"}}"#)
                .unwrap(),
            Frame::Response(_)
        ));
        assert!(matches!(
            CODEC
                .decode(r#"{"method":"session.created","params":{}}"#)
                .unwrap(),
            Frame::Notification(_)
        ));
    }

    #[test]
    fn malformed_and_non_frame_payloads_are_decode_errors() {
        for raw in [
            "",
            "not json",
            "42",
            "[1, 2, 3]",
            r#""bare string""#,
            "null",
            "{}",
        ] {
            let got = CODEC.decode(raw);
            assert!(got.is_err(), "must reject {raw}: got {got:?}");
        }
    }
}
