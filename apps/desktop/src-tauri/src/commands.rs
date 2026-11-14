//! Tauri command surface. Each command is a thin wrapper: parse, delegate to a
//! crate, map the error. If a body grows past a dozen lines, the logic belongs
//! in `crates/`.

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub kernel_pin: String,
}

/// Static facts about the running application, for the About panel and for the
/// front end to confirm it is talking to the build it thinks it is.
#[tauri::command]
pub fn app_info(app: tauri::AppHandle) -> Result<AppInfo, String> {
    let package = app.package_info();
    Ok(AppInfo {
        name: package.name.clone(),
        version: package.version.to_string(),
        kernel_pin: env!("CARGO_PKG_VERSION").to_string(),
    })
}
