#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod server;

use std::{path::PathBuf, sync::Mutex};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    AppHandle, Emitter, Manager, RunEvent, WindowEvent, Wry,
};
#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

const DEFAULT_PORT: u16 = 5421;
const TRAY_TOGGLE_SHARE_ID: &str = "toggle-share";
const TRAY_CHECK_UPDATE_ID: &str = "check-update";
const TRAY_SHOW_ID: &str = "show";
const TRAY_QUIT_ID: &str = "quit";

struct ServerState {
    info: Mutex<Option<server::ServerInfo>>,
    preferred_port: Mutex<u16>,
    tray_menu: Mutex<Option<TrayMenuItems>>,
}

struct TrayMenuItems {
    toggle_share: MenuItem<Wry>,
    check_update: MenuItem<Wry>,
    tray: TrayIcon<Wry>,
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
    app: AppHandle,
) -> Result<server::ServerInfo, String> {
    let info = start_server_inner(port, &state).await?;
    set_tray_share_running(&app, true);
    Ok(info)
}

#[tauri::command]
async fn stop_server(state: tauri::State<'_, ServerState>, app: AppHandle) -> Result<(), String> {
    stop_server_inner(&state).await?;
    set_tray_share_running(&app, false);
    Ok(())
}

#[tauri::command]
fn server_status(
    state: tauri::State<'_, ServerState>,
) -> Result<Option<server::ServerInfo>, String> {
    state
        .info
        .lock()
        .map_err(|error| error.to_string())
        .map(|info| info.clone())
}

#[tauri::command]
async fn check_for_updates(app: AppHandle) -> Result<(), String> {
    check_for_updates_inner(app).await
}

#[tauri::command]
fn set_preferred_port(port: u16, state: tauri::State<'_, ServerState>) -> Result<(), String> {
    if port == 0 {
        return Err("端口号无效".to_string());
    }

    *state
        .preferred_port
        .lock()
        .map_err(|error| error.to_string())? = port;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(ServerState {
            info: Mutex::new(None),
            preferred_port: Mutex::new(DEFAULT_PORT),
            tray_menu: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            pick_admin_files,
            download_admin_file,
            start_server,
            stop_server,
            server_status,
            check_for_updates,
            set_preferred_port
        ])
        .plugin(tauri_plugin_updater::Builder::new().build())
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

            setup_tray(app)?;

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building FileShare")
        .run(|app_handle, event| {
            if let RunEvent::WindowEvent {
                ref label,
                event: WindowEvent::CloseRequested { ref api, .. },
                ..
            } = event
            {
                if label == "main" {
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.hide();
                    }
                    set_dock_visible(app_handle, false);
                }
            }

            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen {
                has_visible_windows,
                ..
            } = event
            {
                if !has_visible_windows {
                    show_main_window(app_handle);
                }
            }
        });
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let toggle_share =
        MenuItem::with_id(app, TRAY_TOGGLE_SHARE_ID, "启动分享", true, None::<&str>)?;
    let check_update =
        MenuItem::with_id(app, TRAY_CHECK_UPDATE_ID, "检查更新", true, None::<&str>)?;
    let show = MenuItem::with_id(app, TRAY_SHOW_ID, "显示窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_ID, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &toggle_share, &check_update, &quit])?;
    let icon = tray_icon().or_else(|| app.default_window_icon().cloned());

    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("FileShare")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_TOGGLE_SHARE_ID => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = toggle_share_from_tray(app.clone()).await {
                        let _ = app.emit("share-error", error);
                    }
                });
            }
            TRAY_CHECK_UPDATE_ID => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    set_tray_update_checking(&app, true);
                    if let Err(error) = check_for_updates_inner(app.clone()).await {
                        show_message(
                            rfd::MessageLevel::Error,
                            "更新失败",
                            &format!("检查更新失败：{error}"),
                        );
                    }
                    set_tray_update_checking(&app, false);
                });
            }
            TRAY_SHOW_ID => show_main_window(app),
            TRAY_QUIT_ID => app.exit(0),
            _ => {}
        });

    if let Some(icon) = icon {
        tray = tray.icon(icon);
        #[cfg(target_os = "macos")]
        {
            tray = tray.icon_as_template(true);
        }
    }

    let tray = tray.build(app)?;
    if let Ok(mut tray_menu) = app.state::<ServerState>().tray_menu.lock() {
        *tray_menu = Some(TrayMenuItems {
            toggle_share: toggle_share.clone(),
            check_update: check_update.clone(),
            tray,
        });
    }
    set_tray_share_running(app.handle(), false);
    Ok(())
}

fn tray_icon() -> Option<Image<'static>> {
    Image::from_bytes(include_bytes!("../icons/tray.png"))
        .ok()
        .map(Image::to_owned)
}

fn inactive_tray_icon() -> Option<Image<'static>> {
    tray_icon().map(|icon| {
        let rgba = icon
            .rgba()
            .chunks_exact(4)
            .flat_map(|pixel| {
                [
                    pixel[0],
                    pixel[1],
                    pixel[2],
                    ((pixel[3] as f32) * 0.38).round() as u8,
                ]
            })
            .collect::<Vec<_>>();
        Image::new_owned(rgba, icon.width(), icon.height())
    })
}

fn show_main_window(app: &tauri::AppHandle) {
    set_dock_visible(app, true);

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn set_dock_visible(app: &tauri::AppHandle, visible: bool) {
    #[cfg(target_os = "macos")]
    {
        let policy = if visible {
            ActivationPolicy::Regular
        } else {
            ActivationPolicy::Accessory
        };
        let _ = app.set_activation_policy(policy);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, visible);
    }
}

async fn toggle_share_from_tray(app: AppHandle) -> Result<(), String> {
    let is_running = app
        .state::<ServerState>()
        .info
        .lock()
        .map_err(|error| error.to_string())?
        .is_some();

    if is_running {
        stop_share_from_tray(app).await
    } else {
        start_share_from_tray(app).await
    }
}

async fn start_share_from_tray(app: AppHandle) -> Result<(), String> {
    let state = app.state::<ServerState>();
    let port = *state
        .preferred_port
        .lock()
        .map_err(|error| error.to_string())?;
    let info = start_server_inner(port, &state).await?;
    set_tray_share_running(&app, true);
    let _ = app.emit("share-started", info);
    Ok(())
}

async fn stop_share_from_tray(app: AppHandle) -> Result<(), String> {
    let state = app.state::<ServerState>();
    stop_server_inner(&state).await?;
    set_tray_share_running(&app, false);
    let _ = app.emit("share-stopped", ());
    Ok(())
}

async fn start_server_inner(port: u16, state: &ServerState) -> Result<server::ServerInfo, String> {
    if port == 0 {
        return Err("端口号无效".to_string());
    }

    *state
        .preferred_port
        .lock()
        .map_err(|error| error.to_string())? = port;

    if let Some(info) = state
        .info
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
    {
        return Ok(info);
    }

    let info = server::start(port).await?;
    *state.info.lock().map_err(|error| error.to_string())? = Some(info.clone());
    Ok(info)
}

async fn stop_server_inner(state: &ServerState) -> Result<(), String> {
    server::stop().await?;
    *state.info.lock().map_err(|error| error.to_string())? = None;
    Ok(())
}

fn set_tray_share_running(app: &AppHandle, running: bool) {
    let items = app
        .state::<ServerState>()
        .tray_menu
        .lock()
        .ok()
        .and_then(|items| {
            items
                .as_ref()
                .map(|items| (items.toggle_share.clone(), items.tray.clone()))
        });

    if let Some((toggle_share, tray)) = items {
        let _ = toggle_share.set_text(if running {
            "停止分享"
        } else {
            "启动分享"
        });
        let icon = if running {
            tray_icon()
        } else {
            inactive_tray_icon()
        };
        let _ = tray.set_icon(icon);
        #[cfg(target_os = "macos")]
        {
            let _ = tray.set_icon_as_template(true);
        }
    }
}

async fn check_for_updates_inner(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    let Some(update) = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?
    else {
        show_message(rfd::MessageLevel::Info, "检查更新", "当前已经是最新版本。");
        return Ok(());
    };

    show_message(
        rfd::MessageLevel::Info,
        "发现新版本",
        "发现新版本，开始下载并安装。安装完成后应用会自动重启。",
    );

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;

    app.restart();
}

fn set_tray_update_checking(app: &AppHandle, checking: bool) {
    let check_update = app
        .state::<ServerState>()
        .tray_menu
        .lock()
        .ok()
        .and_then(|items| items.as_ref().map(|items| items.check_update.clone()));

    if let Some(check_update) = check_update {
        let _ = check_update.set_enabled(!checking);
        let _ = check_update.set_text(if checking {
            "检查中..."
        } else {
            "检查更新"
        });
    }
}

fn show_message(level: rfd::MessageLevel, title: &str, description: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(level)
        .set_title(title)
        .set_description(description)
        .show();
}
