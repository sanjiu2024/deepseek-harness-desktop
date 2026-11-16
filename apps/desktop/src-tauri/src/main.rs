//! Assembling shell. Commands, window setup and plugin registration live here;
//! everything with behaviour lives in `crates/`. See AGENTS.md, "Where code goes".

mod commands;
mod state;
mod workspace;

use std::process::ExitCode;

use state::AppState;
use tauri::Manager;

/// Where the log file lives: the kernel home, beside sessions and settings.
fn log_dir() -> std::path::PathBuf {
    let home = std::env::var_os("DSH_DESKTOP_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| state::app_data_dir().join("dsh-home"));
    home.join("logs")
}

/**
 * stderr plus a daily-rotated file.
 *
 * The returned guard must be held for the life of the process; dropping it stops the writer
 * thread and the last lines of a crash never reach the file — which is precisely the part
 * anyone would want to read.
 */
fn init_tracing() {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "dsh_desktop_app=debug,dsh_runtime=info,dsh_kernel=info,info".into()
        })
    };

    let dir = log_dir();
    // Written synchronously on purpose. A non-blocking appender is faster and loses its buffer
    // when the process is killed — which is exactly the case a log file exists for.
    let file_layer = match std::fs::create_dir_all(&dir) {
        Ok(()) => {
            let appender = tracing_appender::rolling::daily(&dir, "desktop.log");
            Some(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(appender),
            )
        }
        // An unwritable log directory must not stop the app from starting.
        Err(error) => {
            eprintln!(
                "cannot create {}: {error} — logging to stderr only",
                dir.display()
            );
            None
        }
    };

    tracing_subscriber::registry()
        .with(filter())
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .init();
}

/**
 * Send panics through `tracing` as well as stderr.
 *
 * A windowed application has no visible stderr: a panic in a command thread disappears and the
 * user reports "it froze". Writing it through the same logger puts it in the log file, next to
 * the events that led to it.
 */
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown".to_owned());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        tracing::error!(target: "dsh_desktop_app", %location, %message, "panic");
        previous(info);
    }));
}

/** Keep the log directory from growing without bound. */
fn prune_logs(keep_days: u64) {
    let dir = log_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(keep_days * 86_400))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    for entry in entries.flatten() {
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if modified < cutoff {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn main() -> ExitCode {
    // Logs go to stderr **and** to a file: a user reporting a problem cannot reproduce it with
    // RUST_LOG set by hand, and without a file there is nothing to read after the fact.
    init_tracing();
    install_panic_hook();
    // Fourteen days is enough to answer "what happened last week" without becoming a second
    // disk-full problem.
    prune_logs(14);

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
                app.manage(workspace::Terminals::default());
                app.manage(workspace::Watcher::default());
                app.manage(workspace::Previews::default());
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
                    // Shells first: a child process holding the console outlives the window
                    // otherwise, and a second launch inherits the mess.
                    handle.state::<workspace::Terminals>().close_all();

                    let state = handle.state::<AppState>();
                    // Every kernel in the pool, not just the visible one: a leftover process
                    // holds the session lock and the next launch calls it a corrupt store.
                    tauri::async_runtime::block_on(async {
                        state
                            .pool
                            .shutdown_all(std::time::Duration::from_secs(3))
                            .await;
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
