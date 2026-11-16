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

use dsh_runtime::process::{KernelClient, KernelConfig, kernel_entry};
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

    let a1 = pool
        .acquire(&route_a, spawn(template.clone()))
        .await
        .expect("first kernel");
    let a2 = pool
        .acquire(&route_a, spawn(template.clone()))
        .await
        .expect("reuse");
    assert!(
        std::sync::Arc::ptr_eq(&a1, &a2),
        "the same route must reuse its kernel"
    );

    let b = pool
        .acquire(&route_b, spawn(template))
        .await
        .expect("second kernel");
    assert!(
        !std::sync::Arc::ptr_eq(&a1, &b),
        "a different route needs its own process"
    );
    assert_eq!(pool.routes().await.len(), 2);

    pool.bind_session(&route_b, "session-on-b").await;
    assert_eq!(pool.route_of("session-on-b").await, Some(route_b.clone()));
    assert_eq!(pool.route_of("unknown").await, None);

    pool.shutdown_all(Duration::from_secs(5)).await;
    assert!(pool.routes().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dead_kernel_leaves_the_pool() {
    // A fake kernel that exits on its own: the point is that the pool notices, because a
    // route whose process is gone must not keep handing out a client whose stdin is closed.
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/exiting-kernel.mjs");
    let config = KernelConfig {
        node: "node".into(),
        entry: fixture,
        profile: "sdk".into(),
        patches: vec![],
        home: std::env::temp_dir(),
        cwd: std::env::current_dir().expect("cwd"),
        env: Vec::new(),
    };

    let pool = KernelPool::new(2);
    let route = Route::new("fake", "exiting");
    let spawn = move || {
        let config = config.clone();
        async move {
            KernelClient::spawn(config)
                .await
                .map_err(|error| error.to_string())
        }
    };

    let _ = pool.acquire(&route, spawn).await.expect("spawn");
    assert!(pool.existing(&route).await.is_some());

    // The fake exits after 300ms; give the watcher room to run.
    for _ in 0..40 {
        if pool.existing(&route).await.is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the pool still holds a kernel that exited");
}

fn idle_config() -> KernelConfig {
    KernelConfig {
        node: "node".into(),
        entry: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/idle-kernel.mjs"),
        profile: "sdk".into(),
        patches: vec![],
        home: std::env::temp_dir(),
        cwd: std::env::current_dir().expect("cwd"),
        env: Vec::new(),
    }
}

fn idle_spawn()
-> impl FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<KernelClient, String>>>>
{
    let config = idle_config();
    || {
        Box::pin(async move {
            KernelClient::spawn(config)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_oldest_idle_route_is_evicted_at_capacity() {
    // Capacity 2, three routes, none with a session: the least recently used one goes.
    let pool = KernelPool::new(2);
    let a = Route::new("fake", "a");
    let b = Route::new("fake", "b");
    let c = Route::new("fake", "c");

    pool.acquire(&a, idle_spawn()).await.expect("a");
    pool.acquire(&b, idle_spawn()).await.expect("b");
    pool.acquire(&c, idle_spawn()).await.expect("c");

    let routes = pool.routes().await;
    let kept: Vec<String> = routes.iter().map(ToString::to_string).collect();
    pool.shutdown_all(Duration::from_secs(2)).await;

    assert_eq!(
        kept.len(),
        2,
        "capacity is a cap, not a suggestion: {kept:?}"
    );
    assert!(
        !kept.contains(&a.to_string()),
        "the oldest idle route went first: {kept:?}"
    );
    assert!(
        kept.contains(&c.to_string()),
        "the one just asked for stays"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_route_with_a_session_is_never_evicted() {
    // Closing a kernel ends its conversations, so the pool grows rather than doing that.
    let pool = KernelPool::new(1);
    let busy = Route::new("fake", "busy");
    let other = Route::new("fake", "other");

    pool.acquire(&busy, idle_spawn()).await.expect("busy");
    pool.bind_session(&busy, "session-1").await;
    pool.acquire(&other, idle_spawn()).await.expect("other");

    let routes = pool.routes().await;
    let kept: Vec<String> = routes.iter().map(ToString::to_string).collect();
    pool.shutdown_all(Duration::from_secs(2)).await;

    assert_eq!(kept.len(), 2, "capacity was exceeded on purpose: {kept:?}");
    assert!(
        kept.contains(&busy.to_string()),
        "a route with a session survives"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn asking_again_reuses_the_same_kernel() {
    let pool = KernelPool::new(2);
    let route = Route::new("fake", "same");
    let first = pool.acquire(&route, idle_spawn()).await.expect("first");
    let second = pool.acquire(&route, idle_spawn()).await.expect("second");
    let shared = std::sync::Arc::ptr_eq(&first, &second);
    pool.shutdown_all(Duration::from_secs(2)).await;
    assert!(shared, "a route must not get a second process");
}
