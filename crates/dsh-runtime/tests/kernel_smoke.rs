//! End-to-end: a real kernel process, driven by this crate.
//!
//! Skips silently when the kernel has not been fetched — `cargo test` must stay
//! green on a fresh clone. Run `pnpm kernel:fetch` first for the real check.

use std::path::PathBuf;
use std::time::Duration;

use dsh_runtime::{KernelClient, KernelConfig};

fn kernel_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("DSH_KERNEL_DIR") {
        return Some(PathBuf::from(dir));
    }
    // crates/dsh-runtime -> repo root
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let candidate = repo.join("kernel");
    candidate.join("node_modules").exists().then_some(candidate)
}

#[tokio::test]
async fn handshake_reports_the_sdk_runtime_identity() {
    let Some(kernel) = kernel_dir() else {
        eprintln!("skipping: kernel not fetched (run `pnpm kernel:fetch`)");
        return;
    };

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let home = repo.join(".smoke-home-test");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("create scratch home");

    let client = KernelClient::spawn(KernelConfig {
        node: std::env::var_os("DSH_NODE").map_or_else(|| PathBuf::from("node"), PathBuf::from),
        entry: dsh_runtime::kernel_entry(&kernel),
        profile: "sdk".to_owned(),
        patches: Vec::new(),
        home: home.clone(),
        cwd: repo.clone(),
        env: Vec::new(),
    })
    .await
    .expect("kernel spawns");

    let result = client
        .initialize(
            serde_json::json!({
                "cwd": repo.to_string_lossy(),
                "provider": "deepseek-official",
                "model": "deepseek-v4.1-flash",
                "approvalRequests": true,
                "capabilities": { "userQuestions": true },
            }),
            Duration::from_secs(30),
        )
        .await;

    let diagnostics = client.stderr_tail().join("\n");
    let result = result.unwrap_or_else(|error| {
        panic!("initialize failed: {error}\nkernel stderr:\n{diagnostics}")
    });

    assert_eq!(
        result["serverInfo"]["name"], "deepseek-harness-sdk-runtime",
        "unexpected server identity: {result}"
    );

    client.shutdown(Duration::from_secs(5)).await;
    let _ = std::fs::remove_dir_all(&home);
}

/// The live bridge turns assistant-stream frames into stderr lines. This asserts
/// the Rust side parses them into notifications — the half of streaming that the
/// reducer tests cannot see.
///
/// Needs a provider key in the environment; skips without one.
#[tokio::test]
async fn live_bridge_frames_reach_the_event_stream() {
    let Ok(key) = std::env::var("OPENCODE_API_KEY") else {
        eprintln!("skipping: OPENCODE_API_KEY is not set");
        return;
    };
    let Some(kernel) = kernel_dir() else {
        eprintln!("skipping: kernel not fetched (run `pnpm kernel:fetch`)");
        return;
    };

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let home = repo.join(".smoke-live-home");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("node_modules")).expect("create scratch home");

    // Node resolves the bridge by walking up from the profile directory, so the
    // kernel home's own node_modules is where it has to live.
    let bridge = repo.join("bridge");
    let target = home.join("node_modules").join("dsh-desktop-live-bridge");
    std::fs::create_dir_all(&target).expect("create bridge dir");
    for entry in ["index.js", "package.json"] {
        std::fs::copy(bridge.join(entry), target.join(entry)).expect("copy bridge file");
    }

    let client = KernelClient::spawn(KernelConfig {
        node: std::env::var_os("DSH_NODE").map_or_else(|| PathBuf::from("node"), PathBuf::from),
        entry: dsh_runtime::kernel_entry(&kernel),
        profile: "sdk".to_owned(),
        patches: vec![repo.join("profiles/sdk/cordis.patch.yml")],
        home: home.clone(),
        cwd: repo.clone(),
        env: vec![("OPENCODE_API_KEY".to_owned(), key)],
    })
    .await
    .expect("kernel spawns");

    let mut frames = client.subscribe();

    client
        .initialize(
            serde_json::json!({
                "cwd": repo.to_string_lossy(),
                "provider": "opencode-go",
                "model": "deepseek-v4.1-flash",
            }),
            Duration::from_secs(30),
        )
        .await
        .expect("initialize");

    client
        .call(
            "session/prompt",
            serde_json::json!({
                "sessionId": "rust-live-probe",
                "contentBlocks": [{ "type": "text", "text": "数到三，只输出数字。" }],
            }),
            Some(Duration::from_secs(10)),
        )
        .await
        .expect("prompt accepted");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut deltas: Vec<String> = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Ok(frame) = tokio::time::timeout(Duration::from_secs(5), frames.recv()).await else {
            break;
        };
        let Ok(dsh_runtime::Incoming::Notification { method, params }) = frame else {
            continue;
        };
        if method != dsh_runtime::LIVE_CHUNK_METHOD {
            continue;
        }
        if params["kind"] != "chunk" {
            continue;
        }
        if params["chunk"]["type"] == "text-delta" {
            deltas.push(
                params["chunk"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
    }

    let diagnostics = client.stderr_tail().join("\n");
    client.shutdown(Duration::from_secs(5)).await;
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        !deltas.is_empty(),
        "no live text deltas arrived; kernel stderr tail:\n{diagnostics}"
    );
}
