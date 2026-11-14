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

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            app.manage(AppState::resolve(data_dir));
            Ok(())
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
                    let client = state.kernel.lock().expect("kernel lock").take();
                    if let Some(client) = client {
                        tauri::async_runtime::block_on(async move {
                            client.shutdown(std::time::Duration::from_secs(3)).await;
                        });
                    }
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
