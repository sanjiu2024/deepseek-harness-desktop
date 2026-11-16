//! Can the pool actually bring a dead kernel back?
//!
//! The restart policy is unit-tested; this test proves the surrounding mechanics: a kernel that
//! exits leaves the pool, the policy allows exactly one immediate retry, and a second kernel
//! starts on the same route. That is the chain the app leans on when a crash would otherwise mean
//! "restart the application".
//!
//! Skips unless `DSH_DESKTOP_HOME` and `DSH_DESKTOP_KERNEL` are set.

use std::time::Duration;

use dsh_runtime::process::{KernelClient, KernelConfig, kernel_entry};
use dsh_runtime::{KernelPool, RestartPolicy, Route};

fn config(home: &std::path::Path, kernel_dir: &std::path::Path) -> Option<KernelConfig> {
    let entry = kernel_entry(kernel_dir);
    if !entry.exists() {
        eprintln!("skipping: no kernel at {}", entry.display());
        return None;
    }
    Some(KernelConfig {
        node: "node".into(),
        entry,
        profile: "sdk".into(),
        patches: vec![
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../profiles/sdk/cordis.patch.yml"),
        ],
        home: home.to_path_buf(),
        cwd: std::env::current_dir().ok()?,
        env: Vec::new(),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dead_kernel_is_forgotten_and_can_be_restarted() {
    let Ok(home) = std::env::var("DSH_DESKTOP_HOME") else {
        eprintln!("skipping: DSH_DESKTOP_HOME is not set");
        return;
    };
    let Ok(kernel_dir) = std::env::var("DSH_DESKTOP_KERNEL") else {
        eprintln!("skipping: DSH_DESKTOP_KERNEL is not set");
        return;
    };
    let Some(template) = config(
        std::path::Path::new(&home),
        std::path::Path::new(&kernel_dir),
    ) else {
        return;
    };

    let pool = KernelPool::with_policy(2, RestartPolicy::default());
    let route = Route::new("opencode-go", "deepseek-v4.1-flash");
    let spawn = |template: KernelConfig| {
        move || {
            let template = template.clone();
            async move { KernelClient::spawn(template).await }
        }
    };

    let first = pool
        .acquire(&route, spawn(template.clone()))
        .await
        .expect("first kernel starts");
    assert!(pool.existing(&route).await.is_some());

    // Kill it the way a crash would: the exit watcher is the only thing that notices either way.
    first.shutdown(Duration::from_secs(5)).await;

    let mut gone = false;
    for _ in 0..50 {
        if pool.existing(&route).await.is_none() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(gone, "the pool must forget a kernel that exited");

    // The first restart is immediate: nothing has tried this route since it died.
    let delay = pool.restart_delay(&route).await.expect("allowed");
    assert_eq!(delay, Duration::ZERO);

    let second = pool
        .acquire(&route, spawn(template.clone()))
        .await
        .expect("a second kernel starts on the same route");
    assert!(pool.existing(&route).await.is_some());

    // A kernel that answered resets the count, so a later crash is not treated as a loop.
    pool.restarted_successfully(&route).await;
    assert_eq!(pool.restart_count(&route).await, 0);

    second.shutdown(Duration::from_secs(5)).await;
}
