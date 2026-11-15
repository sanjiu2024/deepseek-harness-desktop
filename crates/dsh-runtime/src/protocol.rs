//! The wire vocabulary of the kernel's SDK protocol.
//!
//! Every name here is copied from the upstream protocol package
//! (`@deepseek-ai/dsh-sdk-protocol`). When the pinned kernel version in
//! `profiles/sdk/kernel.json` changes, re-read that package before touching
//! this file — a renamed method is a silent failure, not a compile error.

use serde::{Deserialize, Serialize};

/// Client-to-server request methods.
pub mod method {
    pub const INITIALIZE: &str = "initialize";
    pub const SESSION_PROMPT: &str = "session/prompt";
    pub const SESSION_QUERY: &str = "session/query";
    pub const SESSION_RESUME: &str = "session/resume";
    pub const SESSION_WAIT: &str = "session/wait";
    pub const WORKING_DIRECTORY_GET: &str = "session/working-directory/get";
    pub const WORKING_DIRECTORY_SET: &str = "session/working-directory/set";
    pub const SHUTDOWN: &str = "shutdown";
}

/// Server-to-client requests. The kernel blocks on these until answered, so a
/// client that only listens for notifications deadlocks the agent mid-turn.
pub mod request {
    pub const APPROVAL: &str = "approval/request";
    pub const USER_QUESTION: &str = "user-question/ask";
}

/// Server-to-client notifications.
pub mod notification {
    pub const SESSION_EVENT: &str = "session.event";
    pub const SESSION_STATUS: &str = "session.status";
    pub const SESSION_QUERY_COMPLETED: &str = "session.query.completed";
    pub const SUBAGENT_STARTED: &str = "subagent.started";
    pub const SUBAGENT_FINISHED: &str = "subagent.finished";
    pub const USER_QUESTION_CANCELLED: &str = "user-question.cancelled";
}

/// One decision the user makes on a tool that requires approval.
///
/// Only [`ApprovalOutcome::AllowedOnce`] grants an operation, and only that one
/// operation. There is no standing-consent variant on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalOutcome {
    AllowedOnce,
    Rejected,
    Cancelled,
}

/// One parsed inbound message.
///
/// Serialised for the front end as `{"kind":"notification"|"request"|"response",
/// ...}` so a single listener can switch on one field.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Incoming {
    /// A notification: no id, nothing to answer.
    Notification { method: String, params: serde_json::Value },
    /// A request from the kernel that must be answered, or the agent stalls.
    Request { id: u64, method: String, params: serde_json::Value },
    /// A reply to one of our requests.
    Response { id: u64, payload: ResponsePayload },
}

/// Body of a reply frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ResponsePayload {
    Ok(serde_json::Value),
    Err(RpcError),
}

/// An error the kernel reported for one call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    /// Provider-supplied detail; opaque to us, forwarded to the UI as-is.
    pub data: Option<String>,
}

/// Why a kernel interaction failed.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame is not a JSON object: {0}")]
    NotAnObject(String),
    #[error("frame mixes id and method in a way the protocol forbids: {0}")]
    Ambiguous(String),
    #[error("malformed frame: {0}")]
    Malformed(String),
}

/// Classify one line of kernel output.
///
/// A frame with both `id` and `method` is a request, `id` alone is a response,
/// `method` alone is a notification — the same rule the upstream transport
/// implements.
pub fn parse_frame(line: &str) -> Result<Incoming, ProtocolError> {
    let value: serde_json::Value = serde_json::from_str(line)
        .map_err(|error| ProtocolError::Malformed(format!("{error}: {line}")))?;
    classify(value)
}

/// Classify one already-parsed frame.
pub fn classify(value: serde_json::Value) -> Result<Incoming, ProtocolError> {
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::NotAnObject(value.to_string()))?;

    let id = object.get("id").and_then(serde_json::Value::as_u64);
    let method = object.get("method").and_then(serde_json::Value::as_str);
    let params = object
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    match (id, method) {
        (Some(id), Some(method)) => Ok(Incoming::Request {
            id,
            method: method.to_owned(),
            params,
        }),
        (None, Some(method)) => Ok(Incoming::Notification {
            method: method.to_owned(),
            params,
        }),
        (Some(id), None) => {
            let error = object.get("error");
            let payload = match error {
                Some(error) => ResponsePayload::Err(RpcError {
                    code: error.get("code").and_then(serde_json::Value::as_i64).unwrap_or(0),
                    message: error
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("kernel returned an error with no message")
                        .to_owned(),
                    data: error
                        .get("data")
                        .map(|data| data.to_string()),
                }),
                None => ResponsePayload::Ok(
                    object
                        .get("result")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                ),
            };
            Ok(Incoming::Response { id, payload })
        }
        (None, None) => Err(ProtocolError::Ambiguous(
            "frame has neither id nor method".to_owned(),
        )),
    }
}

/// A request frame, ready to write.
pub fn request_frame(id: u64, method: &str, params: &serde_json::Value) -> String {
    let mut line = format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":");
    push_json_string(&mut line, method);
    line.push_str(",\"params\":");
    line.push_str(&params.to_string());
    line.push('}');
    line
}

/// A response frame answering a kernel request.
pub fn response_frame(id: u64, payload: &ResponsePayload) -> String {
    let mut line = format!("{{\"jsonrpc\":\"2.0\",\"id\":{id}");
    match payload {
        ResponsePayload::Ok(value) => {
            line.push_str(",\"result\":");
            line.push_str(&value.to_string());
        }
        ResponsePayload::Err(error) => {
            line.push_str(",\"error\":{\"code\":");
            line.push_str(&error.code.to_string());
            line.push_str(",\"message\":");
            push_json_string(&mut line, &error.message);
            if let Some(data) = &error.data {
                line.push_str(",\"data\":");
                push_json_string(&mut line, data);
            }
            line.push('}');
        }
    }
    line.push('}');
    line
}

fn push_json_string(out: &mut String, value: &str) {
    // Methods and messages are ASCII in practice, but escaping here keeps a
    // provider error message from breaking the frame.
    match serde_json::to_string(value) {
        Ok(encoded) => out.push_str(&encoded),
        Err(_) => out.push_str("\"\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_outcome_uses_upstream_wire_spelling() {
        let json = serde_json::to_string(&ApprovalOutcome::AllowedOnce).expect("serializes");
        assert_eq!(json, "\"allowed-once\"");
        let json = serde_json::to_string(&ApprovalOutcome::Rejected).expect("serializes");
        assert_eq!(json, "\"rejected\"");
    }

    #[test]
    fn classifies_notification_by_method_alone() {
        let frame = r#"{"jsonrpc":"2.0","method":"session.event","params":{"sessionId":"s"}}"#;
        let Incoming::Notification { method, params } = parse_frame(frame).expect("parses") else {
            panic!("expected a notification");
        };
        assert_eq!(method, "session.event");
        assert_eq!(params["sessionId"], "s");
    }

    #[test]
    fn classifies_request_when_both_id_and_method_are_present() {
        let frame = r#"{"jsonrpc":"2.0","id":7,"method":"approval/request","params":{}}"#;
        let Incoming::Request { id, method, .. } = parse_frame(frame).expect("parses") else {
            panic!("expected a request");
        };
        assert_eq!(id, 7);
        assert_eq!(method, "approval/request");
    }

    #[test]
    fn classifies_response_by_id_alone() {
        let frame = r#"{"jsonrpc":"2.0","id":3,"result":{"serverInfo":{"name":"x"}}}"#;
        let Incoming::Response { id, payload } = parse_frame(frame).expect("parses") else {
            panic!("expected a response");
        };
        assert_eq!(id, 3);
        assert_eq!(payload, ResponsePayload::Ok(serde_json::json!({"serverInfo": {"name": "x"}})));
    }

    #[test]
    fn surfaces_kernel_error_codes_without_losing_the_message() {
        let frame = r#"{"jsonrpc":"2.0","id":9,"error":{"code":-32603,"message":"no adapter registered for provider \"x\""}}"#;
        let Incoming::Response { payload, .. } = parse_frame(frame).expect("parses") else {
            panic!("expected a response");
        };
        let ResponsePayload::Err(error) = payload else {
            panic!("expected an error payload");
        };
        assert_eq!(error.code, -32603);
        assert!(error.message.contains("no adapter registered"));
    }

    #[test]
    fn rejects_a_frame_with_neither_id_nor_method() {
        assert!(matches!(
            parse_frame(r#"{"jsonrpc":"2.0"}"#),
            Err(ProtocolError::Ambiguous(_))
        ));
    }

    #[test]
    fn request_frame_round_trips_through_the_parser() {
        let line = request_frame(42, method::SESSION_PROMPT, &serde_json::json!({"sessionId": "abc"}));
        let Incoming::Request { id, method, params } = parse_frame(&line).expect("parses") else {
            panic!("expected a request");
        };
        assert_eq!(id, 42);
        assert_eq!(method, "session/prompt");
        assert_eq!(params["sessionId"], "abc");
    }

    #[test]
    fn response_frame_escapes_message_quotes() {
        let line = response_frame(
            1,
            &ResponsePayload::Err(RpcError {
                code: -32603,
                message: "bad \"quoted\" text".to_owned(),
                data: None,
            }),
        );
        let Incoming::Response { payload, .. } = parse_frame(&line).expect("parses") else {
            panic!("expected a response");
        };
        let ResponsePayload::Err(error) = payload else {
            panic!("expected an error payload");
        };
        assert_eq!(error.message, "bad \"quoted\" text");
    }
}

#[cfg(test)]
mod wire_shape_tests {
    use super::*;

    /// The front end switches on `kind`. If a variant ever stops carrying it, the
    /// approval dialog silently never appears and the turn hangs — this pins it.
    #[test]
    fn a_request_serialises_with_a_kind_tag() {
        let value = serde_json::to_value(Incoming::Request {
            id: 7,
            method: request::APPROVAL.to_owned(),
            params: serde_json::json!({ "sessionId": "s", "toolName": "pwsh" }),
        })
        .expect("serialises");

        assert_eq!(value["kind"], "request");
        assert_eq!(value["id"], 7);
        assert_eq!(value["method"], "approval/request");
        assert_eq!(value["params"]["toolName"], "pwsh");
    }

    #[test]
    fn a_notification_serialises_with_a_kind_tag() {
        let value = serde_json::to_value(Incoming::Notification {
            method: notification::SESSION_EVENT.to_owned(),
            params: serde_json::json!({}),
        })
        .expect("serialises");

        assert_eq!(value["kind"], "notification");
    }

    #[test]
    fn a_response_serialises_with_a_kind_tag() {
        let value = serde_json::to_value(Incoming::Response {
            id: 1,
            payload: ResponsePayload::Ok(serde_json::json!({})),
        })
        .expect("serialises");

        assert_eq!(value["kind"], "response");
        assert_eq!(value["payload"]["status"], "ok");
    }
}
