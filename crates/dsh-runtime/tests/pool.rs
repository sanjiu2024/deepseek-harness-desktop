//! The pool with real kernels.
//!
//! Skips unless `DSH_DESKTOP_HOME` and a fetched kernel are present, so `cargo test` stays
//! hermetic while a machine with a kernel can prove the routing end to end:
//!
//! ```text
//! DSH_DESKTOP_HOME=…/.dev-home DSH_DESKTOP_KERNEL=…/kernel \
//!   cargo test -p dsh-runtime --test pool -- --nocapture
//! ```

use std::time::Duration;

use dsh_runtime::process::{kernel_entry, KernelClient, KernelConfig};
use dsh_runtime::{KernelPool, Route};

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
async fn two_routes_get_two_kernels_and_one_route_is_reused() {
    let Ok(home) = std::env::var("DSH_DESKTOP_HOME") else {
        eprintln!("skipping: DSH_DESKTOP_HOME is not set");
        return;
    };
    let Ok(kernel_dir) = std::env::var("DSH_DESKTOP_KERNEL") else {
        eprintln!("skipping: DSH_DESKTOP_KERNEL is not set");
        return;
    };
    let home = std::path::PathBuf::from(home);
    let Some(template) = config(&home, std::path::Path::new(&kernel_dir)) else {
        return;
    };

    let pool = KernelPool::new(2);
    let route_a = Route::new("opencode-go", "deepseek-v4.1-flash");
    let route_b = Route::new("deepseek-official", "deepseek-v4.1-pro");

    let spawn = |template: KernelConfig| {
        move || {
            let template = template.clone();
            async move {
                KernelClient::spawn(template)
                    .await
                    .map_err(|error| error.to_string())
            }
        }
    };

    let a1 = pool.acquire(&route_a, spawn(template.clone())).await.expect("first kernel");
    let a2 = pool.acquire(&route_a, spawn(template.clone())).await.expect("reuse");
    assert!(std::sync::Arc::ptr_eq(&a1, &a2), "the same route must reuse its kernel");

    let b = pool.acquire(&route_b, spawn(template)).await.expect("second kernel");
    assert!(!std::sync::Arc::ptr_eq(&a1, &b), "a different route needs its own process");
    assert_eq!(pool.routes().await.len(), 2);

    pool.bind_session(&route_b, "session-on-b").await;
    assert_eq!(pool.route_of("session-on-b").await, Some(route_b.clone()));
    assert_eq!(pool.route_of("unknown").await, None);

    pool.shutdown_all(Duration::from_secs(5)).await;
    assert!(pool.routes().await.is_empty());
}
