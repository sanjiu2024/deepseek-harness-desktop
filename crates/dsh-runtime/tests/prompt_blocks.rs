//! Does the kernel accept an image block this client sends?
//!
//! The shape of `contentBlocks` is a protocol detail decided inside the kernel: it admits the
//! image into its attachment store and rewrites the block to a reference before the model ever
//! sees it. Getting the shape wrong therefore fails *early* — and that is exactly what this test
//! measures. It does not need a model or an API key: a well-formed block is accepted and the turn
//! only fails later, at the model call.
//!
//! Skips unless `DSH_DESKTOP_HOME` and `DSH_DESKTOP_KERNEL` are set.

use std::time::Duration;

use dsh_runtime::process::{KernelClient, KernelConfig, kernel_entry};
use dsh_runtime::prompt::{ContentBlock, text_block};

/// A 1×1 PNG. Small enough to be obvious, real enough to survive a format check.
const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

#[tokio::test(flavor = "multi_thread")]
async fn an_image_block_is_accepted_by_the_kernel() {
    let Ok(home) = std::env::var("DSH_DESKTOP_HOME") else {
        eprintln!("skipping: DSH_DESKTOP_HOME is not set");
        return;
    };
    let Ok(kernel_dir) = std::env::var("DSH_DESKTOP_KERNEL") else {
        eprintln!("skipping: DSH_DESKTOP_KERNEL is not set");
        return;
    };
    let store = dsh_store::SessionStore::new(std::path::PathBuf::from(&home).as_path());
    let Ok(sessions) = store.list() else {
        eprintln!("skipping: cannot read the store");
        return;
    };
    let Some(session) = sessions.iter().find(|session| !session.is_subagent) else {
        eprintln!("skipping: no session — run the app once first");
        return;
    };
    let cwd = session.cwd.clone().unwrap_or_else(|| {
        std::env::current_dir()
            .expect("cwd")
            .to_string_lossy()
            .into_owned()
    });

    // A fresh id: `session/prompt` *creates* the session (it answers "already exists" otherwise),
    // so there is nothing to fork and nothing of the user's to touch. It is removed below.
    let fork_id = format!("blocks-test-{}", std::process::id());
    let client = KernelClient::spawn(KernelConfig {
        node: "node".into(),
        entry: kernel_entry(std::path::Path::new(&kernel_dir)),
        profile: "sdk".into(),
        patches: vec![
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../profiles/sdk/cordis.patch.yml"),
        ],
        home: std::path::PathBuf::from(&home),
        cwd: std::path::PathBuf::from(&cwd),
        env: Vec::new(),
    })
    .await
    .expect("kernel starts");

    client
        .initialize(
            serde_json::json!({
                "cwd": cwd,
                "provider": "opencode-go",
                "model": "deepseek-v4.1-flash",
                "approvalRequests": false,
            }),
            Duration::from_secs(30),
        )
        .await
        .expect("handshake");

    let blocks: Vec<ContentBlock> = vec![
        text_block("图片里是什么？"),
        serde_json::from_value(serde_json::json!({
            "type": "image",
            "mimeType": "image/png",
            "data": PNG_1X1,
            "name": "pixel.png",
        }))
        .expect("block deserialises"),
    ];
    dsh_runtime::prompt::validate(&blocks).expect("our own validation agrees");

    let sent = client
        .call(
            "session/prompt",
            serde_json::json!({ "sessionId": fork_id, "contentBlocks": blocks }),
            Some(Duration::from_secs(30)),
        )
        .await;

    client.shutdown(Duration::from_secs(5)).await;
    let _ = store.purge(&fork_id);

    match sent {
        Ok(value) => println!("[blocks] accepted: {value}"),
        Err(error) => panic!(
            "the kernel rejected a prompt with an image block: {error}\n\n\
             Either the block shape is wrong (the kernel rewrites images into attachment\
             references, so it validates the fields first), or admission failed for another\
             reason — the message says which."
        ),
    }
}
