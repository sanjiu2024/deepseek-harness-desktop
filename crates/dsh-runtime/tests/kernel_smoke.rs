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
