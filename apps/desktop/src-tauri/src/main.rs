//! Assembling shell. Commands, window setup and plugin registration live here;
//! everything with behaviour lives in `crates/`. See AGENTS.md, "Where code goes".

mod commands;
mod state;

use std::process::ExitCode;

use state::AppState;
use tauri::Manager;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "dsh_desktop_app=debug,dsh_runtime=info,dsh_kernel=info,info".into()
            }),
        )
        .init();

    // Create our own data directory before the window exists, so nothing in the
    // startup path depends on a directory appearing later.
    let data_dir = state::app_data_dir();
    if let Err(error) = std::fs::create_dir_all(&data_dir) {
        eprintln!("cannot create {}: {error}", data_dir.display());
        return ExitCode::FAILURE;
    }

    // The WebView2 profile lives beside the kernel home, not in the OS default
    // location. Measured: a process started from inside the harness sandbox may
    // write its workspace and the temp directory but not `%APPDATA%` or
    // `%LOCALAPPDATA%`, so a profile there is created with `EACCES` and WebView2
    // reports the failure as `0x800700AA` / `0x8000FFFF`. Keeping it with the
    // rest of our data makes the location writable wherever the app is allowed
    // to run at all.
    let home = state::dsh_home(&data_dir);
    let profile = home.join("webview2");
    if let Err(error) = std::fs::create_dir_all(&profile) {
        eprintln!("cannot create {}: {error}", profile.display());
        return ExitCode::FAILURE;
    }
    // SAFETY: single-threaded here, before Tauri or any plugin starts a thread
    // that reads the environment.
    unsafe { std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &profile) };

    let result = tauri::Builder::default()
        // Registered first: the single-instance check must run before anything
        // that touches the WebView2 profile. Two processes on one profile is the
        // other way this window fails to appear.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .setup({
            let data_dir = data_dir.clone();
            move |app| {
                app.manage(AppState::resolve(data_dir));
                Ok(())
            }
        })
        .invoke_handler(commands::handlers())
        .build(tauri::generate_context!());

    match result {
        Ok(app) => {
            app.run(|handle, event| {
                // A window close must not leave a Node process holding the
                // session lock, or the next launch reports a corrupt store.
                if let tauri::RunEvent::ExitRequested { .. } = event {
                    let state = handle.state::<AppState>();
                    // Every kernel in the pool, not just the visible one: a leftover process
                    // holds the session lock and the next launch calls it a corrupt store.
                    tauri::async_runtime::block_on(async {
                        state.pool.shutdown_all(std::time::Duration::from_secs(3)).await;
                    });
                }
            });
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to start the application shell: {error}");
            ExitCode::FAILURE
        }
    }
}
