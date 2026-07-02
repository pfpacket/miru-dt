pub mod dtb;
pub mod dts;
pub mod live;
pub mod model;
pub mod phandle;
pub mod render;

use model::LoadResult;
use std::path::Path;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![load_dts, load_dtb, load_live])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
