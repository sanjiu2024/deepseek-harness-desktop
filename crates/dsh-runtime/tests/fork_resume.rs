//! Does the kernel accept a session this client created by copying another one?
//!
//! This is the question that decides whether "fork" is a feature or half of one. Copying a log
//! and rewriting its header only means something if the kernel will resume that session.
//!
//! **It runs against a real kernel home** (`DSH_DESKTOP_HOME`), because the kernel groups
//! sessions by a directory name derived from the session's cwd — a hand-made directory in a fake
//! project folder is not found, and the kernel then reports the *empty* file it created as a
//! corrupt log. The first version of this test learned that the hard way.
//!
//! The fork it creates is removed again, whatever happens.

use std::time::Duration;

use dsh_runtime::process::{KernelClient, KernelConfig, kernel_entry};

#[tokio::test(flavor = "multi_thread")]
async fn the_kernel_resumes_a_forked_session() {
    let Ok(home) = std::env::var("DSH_DESKTOP_HOME") else {
        eprintln!("skipping: DSH_DESKTOP_HOME is not set");
        return;
    };
    let Ok(kernel_dir) = std::env::var("DSH_DESKTOP_KERNEL") else {
        eprintln!("skipping: DSH_DESKTOP_KERNEL is not set");
        return;
    };
    let home = std::path::PathBuf::from(home);
    let entry = kernel_entry(std::path::Path::new(&kernel_dir));
    if !entry.exists() {
        eprintln!("skipping: no kernel at {}", entry.display());
        return;
    }

    let store = dsh_store::SessionStore::new(&home);
    let Ok(sessions) = store.list() else {
        eprintln!("skipping: cannot read the store");
        return;
    };
    let Some(parent) = sessions.iter().find(|session| !session.is_subagent) else {
        eprintln!("skipping: no session to fork — run the app once first");
        return;
    };
    let parent_id = parent.id.clone();
    let cwd = parent.cwd.clone().unwrap_or_else(|| {
        std::env::current_dir()
            .expect("cwd")
            .to_string_lossy()
            .into_owned()
    });

    // A unique id per run: a leftover fork from an interrupted run must not block this one.
    let fork_id = format!("fork-test-{}", std::process::id());
    let fork = match store.fork(&parent_id, &fork_id, Some("fork test")) {
        Ok(fork) => fork,
        Err(error) => {
            eprintln!("skipping: cannot fork {parent_id}: {error}");
            return;
        }
    };

    let client = KernelClient::spawn(KernelConfig {
        node: "node".into(),
        entry,
        profile: "sdk".into(),
        patches: vec![
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../profiles/sdk/cordis.patch.yml"),
        ],
        home: home.clone(),
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

    let resumed = client
        .call(
            "session/resume",
            serde_json::json!({ "sessionId": fork_id }),
            Some(Duration::from_secs(30)),
        )
        .await;

    client.shutdown(Duration::from_secs(5)).await;
    // Always clean up: a test that leaves sessions behind makes the next run's list look wrong.
    let _ = store.purge(&fork_id);

    match resumed {
        Ok(_) => println!(
            "[fork] the kernel resumed {} (copy of {parent_id})",
            fork.id
        ),
        Err(error) => panic!(
            "the kernel refused a forked session: {error}\n\n\
             Copying a log is not enough on its own. Check whether the kernel finds the file at\
             all — it groups sessions by a directory name derived from the cwd, and a copy in the\
             wrong group is reported as a corrupt log rather than a missing session."
        ),
    }
}
