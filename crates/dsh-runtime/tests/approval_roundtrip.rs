//! The approval round trip, without a model or a sandbox.
//!
//! The bug this pins: the kernel issues **string** ids for the requests it sends,
//! and reading only numbers turned every approval into a notification — no dialog,
//! no answer, agent blocked forever. A fake kernel makes the exchange
//! deterministic: send a string-id request, expect the same id back, quoted.

use std::path::PathBuf;
use std::time::Duration;

use dsh_runtime::{ApprovalOutcome, Incoming, KernelClient, KernelConfig};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-kernel.mjs")
}

#[tokio::test]
async fn a_string_id_request_is_answered_with_the_same_id() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

    let client = KernelClient::spawn(KernelConfig {
        node: std::env::var_os("DSH_NODE").map_or_else(|| PathBuf::from("node"), PathBuf::from),
        entry: fixture(),
        profile: "sdk".to_owned(),
        patches: Vec::new(),
        home: repo.join(".fake-home"),
        cwd: repo.clone(),
        env: Vec::new(),
    })
    .await
    .expect("fake kernel spawns");

    let mut frames = client.subscribe();

    client
        .initialize(
            serde_json::json!({ "approvalRequests": true }),
            Duration::from_secs(10),
        )
        .await
        .expect("initialize");

    // The fake sends its approval request right after the handshake.
    let mut answered = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let Ok(Ok(frame)) = tokio::time::timeout(Duration::from_secs(5), frames.recv()).await
        else {
            break;
        };
        if let Incoming::Request { id, method, .. } = frame {
            assert_eq!(method, "approval/request");
            client
                .answer_approval(&id, ApprovalOutcome::AllowedOnce)
                .expect("answer is written");
            answered = true;
            break;
        }
    }

    assert!(
        answered,
        "the fake kernel's request never reached the client"
    );

    client.shutdown(Duration::from_secs(2)).await;

    // The fake exits 0 only when the answer carries the same id back.
    let log = client.stderr_tail();
    assert!(
        log.iter().all(|line| !line.starts_with("FAIL")),
        "fake kernel reported: {log:?}"
    );
}

/// A numeric id from the client still correlates with its response.
#[test]
fn response_frame_keeps_a_numeric_id_bare() {
    use dsh_runtime::{FrameId, ResponsePayload};
    let line = dsh_runtime::protocol::response_frame(
        &FrameId::Number(9),
        &ResponsePayload::Ok(serde_json::json!({ "ok": true })),
    );
    assert!(line.contains("\"id\":9"), "line was {line}");
    assert!(
        !line.contains("\"id\":\"9\""),
        "numeric ids stay numeric: {line}"
    );
}
