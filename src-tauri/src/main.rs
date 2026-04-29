#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod server;

use std::{path::PathBuf, sync::Mutex};

struct ServerState {
    info: Mutex<Option<server::ServerInfo>>,
}

#[tauri::command]
fn pick_admin_files() -> Result<Vec<String>, String> {
    let files = rfd::FileDialog::new()
        .set_directory(dirs::home_dir().unwrap_or_else(std::env::temp_dir))
        .pick_files()
        .unwrap_or_default();
    Ok(files
        .into_iter()
        .map(PathBuf::into_os_string)
        .map(|value| value.to_string_lossy().to_string())
        .collect())
}

#[tauri::command]
async fn download_admin_file(id: String) -> Result<(), String> {
    let suggested_name = server::download_filename(&id).await?;
    let target = rfd::FileDialog::new()
        .set_file_name(&suggested_name)
        .save_file();

    let Some(target_path) = target else {
        return Ok(());
    };

    server::copy_item_to_path(&id, &target_path).await
}

#[tauri::command]
async fn start_server(
    port: u16,
    state: tauri::State<'_, ServerState>,
) -> Result<server::ServerInfo, String> {
    if port == 0 {
        return Err("端口号无效".to_string());
    }

    if let Some(info) = state.info.lock().map_err(|error| error.to_string())?.clone() {
        return Ok(info);
    }

    let info = server::start(port).await?;
    *state.info.lock().map_err(|error| error.to_string())? = Some(info.clone());
    Ok(info)
}

#[tauri::command]
async fn stop_server(state: tauri::State<'_, ServerState>) -> Result<(), String> {
    server::stop().await?;
    *state.info.lock().map_err(|error| error.to_string())? = None;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(ServerState {
            info: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            pick_admin_files,
            download_admin_file,
            start_server,
            stop_server
        ])
        .setup(|app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("admin.html".into()),
            )
            .title("FileShare")
            .inner_size(1100.0, 760.0)
            .min_inner_size(880.0, 560.0)
            .resizable(true)
            .build()?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running FileShare");
}
