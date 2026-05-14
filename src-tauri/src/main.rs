#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod localization;
mod network;
mod server;

use localization::tr;
use serde::Serialize;
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, RunEvent, WindowEvent, Wry,
};
#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

const DEFAULT_PORT: u16 = 5421;
const TRAY_TOGGLE_SHARE_ID: &str = "toggle-share";
const TRAY_CHECK_UPDATE_ID: &str = "check-update";
#[cfg(target_os = "windows")]
const TRAY_ABOUT_ID: &str = "about";
const TRAY_SHOW_ID: &str = "show";
const TRAY_QUIT_ID: &str = "quit";

struct ServerState {
    info: Mutex<Option<server::ServerInfo>>,
    preferred_port: Mutex<u16>,
    update_available: Mutex<bool>,
    tray_menu: Mutex<Option<TrayMenuItems>>,
}

struct TrayMenuItems {
    toggle_share: MenuItem<Wry>,
    check_update: MenuItem<Wry>,
    tray: TrayIcon<Wry>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DragPosition {
    x: f64,
    y: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminFileDropPayload {
    r#type: &'static str,
    paths: Vec<String>,
    position: Option<DragPosition>,
}

#[tauri::command]
fn pick_admin_files() -> Result<Vec<String>, String> {
    let files = rfd::FileDialog::new()
        .set_title(tr("pick_admin_files", &[]))
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
        .set_title(tr("save_file", &[]))
        .set_file_name(&suggested_name)
        .save_file();

    let Some(target_path) = target else {
        return Ok(());
    };

    server::copy_item_to_path(&id, &target_path).await
}

#[tauri::command]
async fn reveal_admin_file(id: String) -> Result<(), String> {
    let path = server::item_file_path(&id).await?;
    reveal_file(&path)
}

fn reveal_file(path: &PathBuf) -> Result<(), String> {
    if !path.exists() {
        return Err(tr("source_missing", &[]));
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .status()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .status()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let directory = path.parent().unwrap_or(path.as_path());
        std::process::Command::new("xdg-open")
            .arg(directory)
            .status()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err(tr("not_supported", &[]))
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
fn has_update_available(state: tauri::State<'_, ServerState>) -> Result<bool, String> {
    state
        .update_available
        .lock()
        .map_err(|error| error.to_string())
        .map(|value| *value)
}

#[tauri::command]
fn set_preferred_port(port: u16, state: tauri::State<'_, ServerState>) -> Result<(), String> {
    if port == 0 {
        return Err(tr("invalid_port", &[]));
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
            update_available: Mutex::new(false),
            tray_menu: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            pick_admin_files,
            download_admin_file,
            reveal_admin_file,
            start_server,
            stop_server,
            server_status,
            check_for_updates,
            has_update_available,
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

            // 启动时在后台检查更新
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                let _ = check_for_updates_on_startup(app_handle).await;
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building FileShare")
        .run(|app_handle, event| {
            if let RunEvent::WindowEvent {
                ref label, ref event, ..
            } = event
            {
                if label == "main" {
                    match event {
                        WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.hide();
                            }
                            set_dock_visible(app_handle, false);
                        }
                        WindowEvent::DragDrop(drop_event) => {
                            emit_admin_file_drop(app_handle, drop_event);
                        }
                        _ => {}
                    }
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
    let toggle_share = MenuItem::with_id(
        app,
        TRAY_TOGGLE_SHARE_ID,
        tr("start_share", &[]),
        true,
        None::<&str>,
    )?;
    let check_update = MenuItem::with_id(
        app,
        TRAY_CHECK_UPDATE_ID,
        tr("check_update", &[]),
        true,
        None::<&str>,
    )?;
    #[cfg(target_os = "windows")]
    let about = MenuItem::with_id(app, TRAY_ABOUT_ID, "关于", true, None::<&str>)?;
    let show = MenuItem::with_id(app, TRAY_SHOW_ID, "显示窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_ID, "退出", true, None::<&str>)?;
    #[cfg(target_os = "windows")]
    let menu = Menu::with_items(app, &[&show, &toggle_share, &check_update, &about, &quit])?;
    #[cfg(not(target_os = "windows"))]
    let menu = Menu::with_items(app, &[&show, &toggle_share, &check_update, &quit])?;
    let icon = platform_tray_icon().or_else(|| app.default_window_icon().cloned());

    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("FileShare")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
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
                            &tr("update_failed", &[]),
                            &format!("{}: {error}", tr("update_failed", &[])),
                        );
                    }
                    set_tray_update_checking(&app, false);
                });
            }
            TRAY_SHOW_ID => show_main_window(app),
            #[cfg(target_os = "windows")]
            TRAY_ABOUT_ID => show_about_dialog(app),
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

fn emit_admin_file_drop(app: &AppHandle, event: &tauri::DragDropEvent) {
    eprintln!("FileShare drag/drop event: {event:?}");

    if let tauri::DragDropEvent::Drop { paths, .. } = event {
        let app_handle = app.clone();
        let dropped_paths = paths.to_vec();
        tauri::async_runtime::spawn(async move {
            match server::add_admin_local_files(dropped_paths).await {
                Ok(count) => {
                    eprintln!("FileShare registered {count} dropped file(s)");
                    let _ = app_handle.emit("admin-files-added", count);
                }
                Err(error) => {
                    eprintln!("FileShare failed to register dropped file(s): {error}");
                    let _ = app_handle.emit("share-error", error);
                }
            }
        });
    }

    let payload = match event {
        tauri::DragDropEvent::Enter { paths, position } => AdminFileDropPayload {
            r#type: "enter",
            paths: drag_paths(paths),
            position: Some(DragPosition {
                x: position.x,
                y: position.y,
            }),
        },
        tauri::DragDropEvent::Over { position } => AdminFileDropPayload {
            r#type: "over",
            paths: Vec::new(),
            position: Some(DragPosition {
                x: position.x,
                y: position.y,
            }),
        },
        tauri::DragDropEvent::Drop { paths, position } => AdminFileDropPayload {
            r#type: "drop",
            paths: drag_paths(paths),
            position: Some(DragPosition {
                x: position.x,
                y: position.y,
            }),
        },
        tauri::DragDropEvent::Leave => AdminFileDropPayload {
            r#type: "leave",
            paths: Vec::new(),
            position: None,
        },
        _ => return,
    };

    let _ = app.emit("admin-file-drop", payload);
}

fn drag_paths(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path.as_os_str().to_string_lossy().to_string())
        .collect()
}

fn tray_icon() -> Option<Image<'static>> {
    Image::from_bytes(include_bytes!("../icons/tray.png"))
        .ok()
        .map(Image::to_owned)
}

#[cfg(target_os = "windows")]
fn platform_tray_icon() -> Option<Image<'static>> {
    Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .ok()
        .map(Image::to_owned)
}

#[cfg(not(target_os = "windows"))]
fn platform_tray_icon() -> Option<Image<'static>> {
    tray_icon()
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
                    ((pixel[3] as f32) * 0.70).round() as u8,
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
        return Err(tr("invalid_port", &[]));
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
            tr("stop_share", &[])
        } else {
            tr("start_share", &[])
        });
        #[cfg(target_os = "macos")]
        let icon = if running { tray_icon() } else { inactive_tray_icon() };
        #[cfg(not(target_os = "macos"))]
        let icon = platform_tray_icon();
        let _ = tray.set_icon(icon);
        #[cfg(target_os = "macos")]
        {
            let _ = tray.set_icon_as_template(true);
        }
    }
}

async fn check_for_updates_on_startup(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    println!("启动时检查更新...");
    let update = app
        .updater_builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;

    if update.is_some() {
        println!("启动时发现新版本");
        if let Some(state) = app.try_state::<ServerState>() {
            if let Ok(mut available) = state.update_available.lock() {
                *available = true;
            }
        }
        let _ = app.emit("update-available", ());
    }

    Ok(())
}

async fn check_for_updates_inner(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    println!("开始检查更新...");

    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| {
            let err_msg = format!("创建updater失败: {}", error);
            println!("{}", err_msg);
            err_msg
        })?;

    println!("正在检查更新...");

    let update = updater
        .check()
        .await
        .map_err(|error| {
            let err_msg = format!("检查更新失败: {}", error);
            println!("{}", err_msg);
            err_msg
        })?;

    let Some(update) = update else {
        println!("当前已是最新版本");
        if let Some(state) = app.try_state::<ServerState>() {
            if let Ok(mut available) = state.update_available.lock() {
                *available = false;
            }
        }
        show_message(
            rfd::MessageLevel::Info,
            &tr("check_update", &[]),
            &tr("latest_version", &[]),
        );
        return Ok(());
    };

    println!("检查更新发现新版本: {}", update.version);
    if let Some(state) = app.try_state::<ServerState>() {
        if let Ok(mut available) = state.update_available.lock() {
            *available = true;
        }
    }

    let current_version = app.package_info().version.to_string();
    let new_version = update.version.clone();
    let update_body = update.body.clone().unwrap_or_default();

    println!("发现新版本: {} (当前: {})", new_version, current_version);

    let message = format!(
        "发现新版本 {}\n当前版本: {}\n\n{}\n\n是否立即下载并安装？安装完成后应用将自动重启。",
        new_version,
        current_version,
        if update_body.is_empty() { "查看 GitHub 发布页面了解更新内容" } else { &update_body }
    );

    let result = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(tr("new_version", &[]))
        .set_description(&message)
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show();

    if !matches!(result, rfd::MessageDialogResult::Ok) {
        println!("用户取消更新");
        return Ok(());
    }

    println!("开始下载更新...");

    update
        .download_and_install(
            |chunk_length, content_length| {
                if let Some(total) = content_length {
                    let progress = (chunk_length as f64 / total as f64 * 100.0) as u32;
                    println!("下载进度: {}%", progress);
                }
            },
            || {
                println!("下载完成，开始安装...");
            },
        )
        .await
        .map_err(|error| {
            let err_msg = format!("下载或安装失败: {}", error);
            println!("{}", err_msg);
            err_msg
        })?;

    println!("安装完成，准备重启...");
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
            tr("checking", &[])
        } else {
            tr("check_update", &[])
        });
    }
}

#[cfg(target_os = "windows")]
fn show_about_dialog(app: &AppHandle) {
    let version = app.package_info().version.to_string();
    let description = tr("about_desc", &[("version", version)]);

    show_message(rfd::MessageLevel::Info, &tr("about_title", &[]), &description);
}

fn show_message(level: rfd::MessageLevel, title: &str, description: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(level)
        .set_title(title)
        .set_description(description)
        .show();
}
