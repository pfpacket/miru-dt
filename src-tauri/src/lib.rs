pub mod dtb;
pub mod dts;
pub mod live;
pub mod model;
pub mod phandle;
pub mod render;

use model::LoadResult;
use std::path::Path;
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
async fn load_dts(path: String, include_dirs: Vec<String>) -> Result<LoadResult, String> {
    dts::parse_dts_file(Path::new(&path), &include_dirs)
}

#[tauri::command]
async fn load_dtb(path: String) -> Result<LoadResult, String> {
    dtb::load(Path::new(&path))
}

#[tauri::command]
async fn load_live(path: Option<String>) -> Result<LoadResult, String> {
    live::load(path.as_deref().unwrap_or("/proc/device-tree"))
}

/// Open a source file in the user's editor, at `line` when the editor
/// supports it. Resolution order: the MIRU_DT_EDITOR env var (a shell
/// command template with `{file}` and `{line}` placeholders), then VS Code
/// (`code --goto`), then the system default application.
#[tauri::command]
async fn open_source(app: tauri::AppHandle, file: String, line: Option<u32>) -> Result<(), String> {
    let line = line.unwrap_or(1);
    if let Ok(template) = std::env::var("MIRU_DT_EDITOR") {
        let cmd = template
            .replace("{file}", &file)
            .replace("{line}", &line.to_string());
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .spawn()
            .map_err(|e| format!("MIRU_DT_EDITOR command failed: {e}"))?;
        return Ok(());
    }
    if std::process::Command::new("code")
        .arg("--goto")
        .arg(format!("{file}:{line}"))
        .spawn()
        .is_ok()
    {
        return Ok(());
    }
    app.opener()
        .open_path(&file, None::<&str>)
        .map_err(|e| format!("cannot open {file}: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_dts,
            load_dtb,
            load_live,
            open_source
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
