#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod apps;
mod mapping;
mod remote;

/// 必须与 tauri.conf.json 的 identifier 一致、以及产物 Info.plist 的 CFBundleIdentifier 一致。
/// 三者不一致时 macOS TCC 会把它当成另一个 App，已授权的权限就"消失"了。
pub const BUNDLE_ID: &str = "com.webcoding.desktop";

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn wc_hid_request_listen_event_access();
}

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::ShortcutState;
use tauri_plugin_updater::UpdaterExt;

use mapping::engine::{Config, Engine, Mapping, MappingKind, ActionResult};

pub struct AppState {
    pub engine: Mutex<Engine>,
    /// 持有 mdns daemon，防止 drop 后 Bonjour 服务注销
    pub mdns: Mutex<Option<mdns_sd::ServiceDaemon>>,
    pub server_port: AtomicU16,
    pub tv_connected: AtomicBool,
    // A2854 蓝牙直连状态
    pub connected_devices: Mutex<usize>,
    /// 在系统蓝牙里见过的 A2854 标签。它只能由扫描结果写入 ——
    /// 应用内无法配对，所以「已配对」这件事只能由 system_profiler 的读数旁证。
    pub paired_device: Mutex<Option<String>>,
    /// 最近一次扫描到的设备列表
    pub bt_devices: Mutex<Vec<remote::bluetooth::BtDevice>>,
    /// 连接状态快照（十值机 + 三层读数），由状态机线程刷新
    pub connection_state: Mutex<remote::connection::ConnectionState>,
    pub last_hid: Mutex<Option<remote::hid::HidEvent>>,
    pub hid_error: Mutex<Option<String>>,
    pub last_event: Mutex<Option<String>>,
    /// 最近 50 条派发结果（最新在前），随 diag.json 落盘供排障
    pub recent_results: Mutex<Vec<mapping::engine::ActionResult>>,
    /// 原始 HID 样本抓取（raw / touch*），随 diag.json 落盘。
    /// 触摸板报文形态未知，先全量记录再定算法 —— 不给「没生效」留猜的空间。
    pub raw_samples: Mutex<Vec<serde_json::Value>>,
    pub raw_total: std::sync::atomic::AtomicU64,
}

fn config_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("webcoding-config.json")
}

// ---------- Tauri commands ----------

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.engine.lock().unwrap().config.clone()
}

#[tauri::command]
fn set_touchpad(state: State<AppState>, config: mapping::engine::TouchpadConfig) -> Result<(), String> {
    if !config.gain.is_finite() || !(0.1..=30.0).contains(&config.gain) {
        return Err("灵敏度必须在 0.1–30 之间".into());
    }
    state.engine.lock().unwrap().set_touchpad(config)
}

#[tauri::command]
fn set_mapping(
    state: State<AppState>,
    preset: String,
    event: String,
    kind: String,
    value: String,
) -> Result<(), String> {
    let parsed = MappingKind::parse(&kind).ok_or_else(|| format!("未知动作类型: {kind}"))?;
    state
        .engine
        .lock()
        .unwrap()
        .set_mapping(&preset, &event, Mapping { kind: parsed, value })
}

#[tauri::command]
fn set_preset(state: State<AppState>, name: String) -> Result<(), String> {
    state.engine.lock().unwrap().set_preset(&name)
}

#[tauri::command]
fn add_preset(state: State<AppState>, name: String) -> Result<(), String> {
    state.engine.lock().unwrap().add_preset(&name)
}

#[tauri::command]
fn delete_preset(state: State<AppState>, name: String) -> Result<(), String> {
    state.engine.lock().unwrap().delete_preset(&name)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    touchpad: serde_json::Value,
    running: bool,
    port: u16,
    tv_connected: bool,
    accessibility: bool,
    front_app: String,
    connected_devices: usize,
    hid_error: Option<String>,
    last_event: Option<String>,
}

fn status_snapshot(state: &State<AppState>) -> Status {
    let port = state.server_port.load(Ordering::Relaxed);
    Status {
        touchpad: remote::touchpad::diagnostics(),        running: port != 0,
        port,
        tv_connected: state.tv_connected.load(Ordering::Relaxed),
        accessibility: actions::keyboard::accessibility_granted(),
        front_app: apps::detector::frontmost_app().unwrap_or_default(),
        connected_devices: *state.connected_devices.lock().unwrap(),
        hid_error: state.hid_error.lock().unwrap().clone(),
        last_event: state.last_event.lock().unwrap().clone(),
    }
}

#[tauri::command]
fn get_status(state: State<AppState>) -> Status {
    status_snapshot(&state)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInput {
    preset: String,
    #[serde(default)]
    target: String,
}

/// 设置某个 Preset 的目标 App；留空表示回到「发给当前前台 App」
#[tauri::command]
fn set_preset_target(state: State<AppState>, input: TargetInput) -> Result<(), String> {
    state
        .engine
        .lock()
        .unwrap()
        .set_preset_target(&input.preset, &input.target)
}

#[tauri::command]
fn last_hid_event(state: State<AppState>) -> Option<remote::hid::HidEvent> {
    state.last_hid.lock().unwrap().clone()
}

/// 系统诊断：权限与 HID 全部由 Rust / Native 层真实读取，前端不自行推断
#[tauri::command]
fn get_system_status(state: State<AppState>) -> remote::hid::SystemStatus {
    remote::hid::system_status(&state)
}

/// 十值连接状态 + 三层独立读数：全部由 Rust 侧真实读取，前端只渲染
#[tauri::command]
fn get_connection_state(state: State<AppState>) -> remote::connection::ConnectionState {
    state.connection_state.lock().unwrap().clone()
}

/// 最近一次扫描结果，UI 在「发现设备」步骤直接渲染
#[tauri::command]
fn get_found_devices(state: State<AppState>) -> Vec<remote::bluetooth::BtDevice> {
    state.bt_devices.lock().unwrap().clone()
}

/// 搜索附近的蓝牙设备（走 system_profiler 子进程，不会碰 TCC）
#[tauri::command]
fn bt_start_scan(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    *state.bt_devices.lock().unwrap() = Vec::new();
    remote::bluetooth::start_scan(&app);
    remote::connection::push(remote::connection::Signal::ScanStarted);
    Ok(())
}

#[tauri::command]
fn bt_stop_scan() {
    remote::bluetooth::stop_scan();
}

/// 唯一可行的配对 / 连接入口：系统蓝牙设置页。
/// 应用内无法配对 —— 进程内碰蓝牙 API 会被 macOS TCC 直接 abort（见 bluetooth.rs）。
#[tauri::command]
fn open_bluetooth_settings() {
    let _ = actions::shell::run(
        "open 'x-apple.systempreferences:com.apple.preference.bluetooth'",
    );
}

/// 走系统授权流程（IOHIDRequestAccess），比直接翻系统设置更准：
/// 用户可以在弹窗里直接授权，也会被记录在 TCC 里。
#[tauri::command]
fn request_input_monitoring() {
    #[cfg(target_os = "macos")]
    unsafe {
        wc_hid_request_listen_event_access();
    }
}

/// 兜底：直接打开「隐私与安全性 → 输入监控」设置页
#[tauri::command]
fn open_input_monitoring_settings() {
    let _ = actions::shell::run(
        "open 'x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent'",
    );
}

/// 打开「隐私与安全性 → 辅助功能」
#[tauri::command]
fn request_accessibility_settings() {
    let _ = actions::shell::run(
        "open 'x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility'",
    );
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
struct KindInput {
    kind: String,
    #[serde(default)]
    value: String,
}

#[tauri::command]
async fn test_action(action: KindInput) -> Result<ActionResult, String> {
    let kind = MappingKind::parse(&action.kind)
        .ok_or_else(|| format!("未知动作类型: {}", action.kind))?;
    let mapping = Mapping {
        kind,
        value: action.value,
    };
    tauri::async_runtime::spawn_blocking(move || Ok(mapping::engine::dispatch("test", &mapping)))
        .await
        .map_err(|e| e.to_string())?
}

// ---------- Tray / window ----------

/// 按住窗口空白处拖动窗口。
/// `data-tauri-drag-region` 只在命中元素自身时生效，卡片/表格等子元素拖不动，
/// 所以前端在 mousedown 时直接调这里，不依赖属性匹配。
#[tauri::command]
fn start_dragging(window: tauri::Window) {
    let _ = window.start_dragging();
}

/// 用系统默认浏览器打开外部链接（关于页「检查更新 / 反馈」入口）。
/// 冷路径，允许 spawn 子进程；热路径（遥控事件）禁止这么做。
#[tauri::command]
fn open_url(url: String) {
    #[cfg(target_os = "macos")]
    {
        // 只放行 http/https，避免被拼进任意 scheme
        if url.starts_with("https://") || url.starts_with("http://") {
            let _ = std::process::Command::new("open").arg(&url).spawn();
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = url;
}

/// 重启应用。Tauri 2 没有给 core process 插件注册 restart 权限，
/// 更新安装完成后直接在这里走 app.restart()，前端不用额外插件。
#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

/// 应用版本号（取自 tauri.conf.json 的 version），关于页展示用。
#[tauri::command]
fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 隐藏 macOS 红绿灯里的「放大」按钮，只保留关闭 + 最小化。
/// Overlay 标题栏模式下 tauri 没有现成 API，直接 objc FFI NSWindow。
#[cfg(target_os = "macos")]
fn hide_zoom_button(window: &tauri::WebviewWindow) {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_void};

    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        fn sel_registerName(name: *const c_char) -> *const c_char;
        #[link_name = "objc_msgSend"]
        fn msg_send_std_button(receiver: *mut c_void, sel: *const c_char, which: *mut c_void) -> *mut c_void;
        #[link_name = "objc_msgSend"]
        fn msg_send_set_hidden(receiver: *mut c_void, sel: *const c_char, flag: bool);
    }

    unsafe {
        let Ok(ns) = window.ns_window() else { return };
        let ns = ns as *mut c_void;
        // SEL 必须走 sel_registerName 注册，C 字符串指针不能直接当 SEL
        let sel_std = sel_registerName(CString::new("standardWindowButton:").unwrap().as_ptr());
        let sel_hide = sel_registerName(CString::new("setHidden:").unwrap().as_ptr());
        // NSWindowButton: closeButton = 0, miniaturizeButton = 1, zoomButton = 2
        let zoom = msg_send_std_button(ns, sel_std, 2 as *mut c_void);
        if !zoom.is_null() {
            msg_send_set_hidden(zoom, sel_hide, true);
        }
    }
}

fn toggle_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// 运行时生成托盘图标（灰底白点），避免依赖打包 icon 资源
fn tray_icon_image() -> tauri::image::Image<'static> {
    let (w, h) = (32u32, 32u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let cx = (w - 1) as f32 / 2.0;
    let cy = (h - 1) as f32 / 2.0;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= cx * cy {
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[0x2C, 0x2C, 0x2A, 0xFF]);
            }
        }
    }
    for y in 13..19 {
        for x in 13..19 {
            let i = ((y * w + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        }
    }
    tauri::image::Image::new_owned(rgba, w, h)
}

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let toggle = MenuItem::with_id(app, "toggle", "显示 / 隐藏设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 VibeRemote", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &quit])?;

    TrayIconBuilder::new()
        .icon(tray_icon_image())
        .tooltip("VibeRemote")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => toggle_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["cmd+shift+w"]).expect("注册全局快捷键 Cmd+Shift+W 失败")
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        // 内置更新：自行下载安装包并替换应用，不跳转 GitHub
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            let engine = Engine::load(config_path(&handle));
            let state = AppState {
                engine: Mutex::new(engine),
                mdns: Mutex::new(None),
                server_port: AtomicU16::new(0),
                tv_connected: AtomicBool::new(false),
                connected_devices: Mutex::new(0),
                paired_device: Mutex::new(None),
                bt_devices: Mutex::new(Vec::new()),
                connection_state: Mutex::new(
                    remote::connection::ConnectionState::build(
                        remote::connection::RemoteMachine::default(),
                        remote::connection::Readings::default(),
                        None,
                    ),
                ),
                last_hid: Mutex::new(None),
                hid_error: Mutex::new(None),
                last_event: Mutex::new(None),
                recent_results: Mutex::new(Vec::new()),
                raw_samples: Mutex::new(Vec::new()),
                raw_total: std::sync::atomic::AtomicU64::new(0),
            };
            // 两个输入源互相独立：tvOS WebSocket（局域网）与 A2854 蓝牙 HID 直连
            remote::server::start(handle.clone());
            app.manage(state);
            let managed = app.state::<AppState>();
            // 蓝牙 / HID / 权限三层的状态机先跑起来，UI 一进来就有真实读数
            remote::connection::start(&handle);
            // 诊断快照落盘：排「按键没反应」时直接读 diag.json，不用让用户截图转述
            remote::diag::start(handle.clone());
            remote::touchpad::start(handle.clone());
            if let Err(e) = remote::hid::start(handle.clone(), &managed) {
                *managed.hid_error.lock().unwrap() = Some(e.clone());
                let _ = handle.emit("siri-remote-error", e);
            }
            // Overlay 标题栏：隐藏放大按钮，只留关闭 + 最小化
            if let Some(window) = app.get_webview_window("main") {
                hide_zoom_button(&window);
            }
            setup_tray(&handle)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 后台常驻：关闭窗口 = 隐藏，托盘/快捷键可随时唤回
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_dragging,
            get_config,
            set_mapping,
            set_preset,
            set_touchpad,
            add_preset,
            delete_preset,
            get_status,
            test_action,
            request_accessibility_settings,
            request_input_monitoring,
            open_input_monitoring_settings,
            get_system_status,
            last_hid_event,
            get_connection_state,
            get_found_devices,
            bt_start_scan,
            bt_stop_scan,
            open_bluetooth_settings,
            set_preset_target,
            open_url,
            get_app_version,
            restart_app
        ])
        .run(tauri::generate_context!())
        .expect("VibeRemote 启动失败");
}
