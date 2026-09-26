//! A2854（第三代 Siri Remote / USB-C）蓝牙 HID 直连。
//!
//! 与 tvOS / WebSocket 输入源完全对等：都只产出「遥控事件字符串」，
//! 再交给 Mapping Engine 分派。这样加任何新外设（Stream Deck、手柄、网页遥控器）都不用改动作层。

use serde::Serialize;
use std::os::raw::{c_char, c_int, c_long, c_uint};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

/// 确认键按住超过该时长判定为长按（centerLongPress）
const LONG_PRESS: Duration = Duration::from_millis(600);
/// A2854 会把同一个逻辑按钮通过多个 HID 接口重复上报，25ms 内的相同状态变化直接丢弃
const DEDUPE: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HidEvent {
    pub event: String,
    pub usage_page: u32,
    pub usage: u32,
    pub value: i64,
    pub pressed: bool,
}

#[derive(Debug, Clone)]
struct NativeEvent {
    name: String,
    usage_page: u32,
    usage: u32,
    value: i64,
}

static EVENT_TX: OnceLock<std::sync::mpsc::Sender<NativeEvent>> = OnceLock::new();
static CONN_TX: OnceLock<std::sync::mpsc::Sender<i32>> = OnceLock::new();

/// 权限状态轮询间隔。用户在系统设置里勾选开关时 App 无法感知，
/// 只能靠轮询发现；3 秒足够「授权即生效」。
const PERMISSION_RETRY: Duration = Duration::from_secs(3);

/// Apple TV 遥控器的 VID / PID 白名单（与 native/siri_remote_bridge.c 保持一致）。
///
/// ⚠️ 别把 PID 当成常量写死成某一个值：A2854（USB-C 第三代）实测是 **0x0314**，
/// 而 A2540（第二代）才是 0x0315。只认 0x0315 会一台都匹配不上，
/// 表现是「遥控器能控制 Mac，但 App 收不到按键」。这四项来自
/// `com.apple.driver.AppleBluetoothRemote` 的 `ProductIDArray = (614, 621, 788, 789)`。
pub const VENDOR_ID: &str = "0x004C";
pub const PRODUCT_ID_WHITELIST: [&str; 4] = ["0x0266", "0x026D", "0x0314", "0x0315"];

fn hex_pid(pid: i32) -> String {
    format!("0x{pid:04X}")
}

/// 真实权限状态，三态。来源必须是 macOS HID 权限 API，
/// 不允许前端自己猜，也不允许用 IOHIDManagerOpen 的失败结果反推。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionState {
    /// 已授权
    Granted,
    /// 明确拒绝
    Denied,
    /// 还没请求过
    Unknown,
}

impl PermissionState {
    fn from_raw(value: c_int) -> Self {
        match value {
            1 => PermissionState::Granted,
            2 => PermissionState::Denied,
            _ => PermissionState::Unknown,
        }
    }
}

/// 系统诊断快照，由 Rust 侧真实读取后返回给前端
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub input_monitoring: PermissionState,
    pub accessibility: PermissionState,
    pub hid_manager_open: bool,
    /// HID 输入回调是否真的在跑
    pub input_callback_active: bool,
    pub a2854_matched: bool,
    pub matched_device_count: usize,
    pub vendor_id: String,
    /// 实际匹配到的遥控器 PID；未匹配时为 `None`（不要回填期望值，那是撒谎）
    pub product_id: Option<String>,
    /// 匹配白名单，未匹配时用来告诉用户「到底在等什么」
    pub product_id_whitelist: Vec<String>,
    /// 当前真正执行的二进制路径，用于核对 TCC 权限记在了哪个 App 上
    pub app_path: String,
    /// Tauri identifier，与 Info.plist 的 CFBundleIdentifier 应一致
    pub bundle_id: &'static str,
    pub last_hid_event: Option<HidEvent>,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn wc_siri_remote_start(
        event_cb: extern "C" fn(*const c_char, c_uint, c_uint, c_long),
        connection_cb: extern "C" fn(c_int),
    ) -> c_int;
    /// 停掉当前监听并重新注册（复用已保存的回调），用于补授权后重试
    fn wc_siri_remote_restart();
    fn wc_siri_remote_connected_count() -> c_int;
    /// 实际匹配到的遥控器 PID（0 = 还没匹配到）
    fn wc_siri_remote_matched_pid() -> c_int;
    /// 真实权限查询：1 granted / 2 denied / 0 unknown
    fn wc_hid_listen_event_access() -> c_int;
    /// 触发系统授权弹窗
    fn wc_hid_request_listen_event_access();
    fn wc_hid_manager_open() -> c_int;
    fn wc_hid_callback_active() -> c_int;
}

extern "C" fn native_event_cb(name: *const c_char, page: c_uint, usage: c_uint, value: c_long) {
    if name.is_null() {
        return;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned();
    if let Some(tx) = EVENT_TX.get() {
        let _ = tx.send(NativeEvent {
            name,
            usage_page: page,
            usage,
            value: value as i64,
        });
    }
}

extern "C" fn native_connection_cb(count: c_int) {
    if let Some(tx) = CONN_TX.get() {
        let _ = tx.send(count);
    }
}

/// HID 桥接产出的物理名 → 项目内部的遥控事件名
fn to_remote_event(name: &str) -> Option<&'static str> {
    Some(match name {
        "up" => "up",
        "down" => "down",
        "left" => "left",
        "right" => "right",
        "select" => "center",
        "back" | "tv" => "back",
        "playPause" => "playPause",
        "siri" => "siri",
        // 音量 / 静音 / 电源 / 未知 usage：不占用映射槽位，但仍进入诊断流
        "volumeUp" | "volumeDown" | "mute" | "power" | "raw" | "permissionError" | "hidOpenError" => {
            return None
        }
        _ => return None,
    })
}

/// 进程内只跑一次 HID 监听。
static HID_STARTED: AtomicBool = AtomicBool::new(false);

pub fn start(app: AppHandle, state: &tauri::State<AppState>) -> Result<(), String> {
    if cfg!(not(target_os = "macos")) {
        *state.hid_error.lock().unwrap() =
            Some("A2854 直连 HID 输入仅在 macOS 上可用".into());
        return Ok(());
    }
    if HID_STARTED.swap(true, Ordering::SeqCst) {
        // 已经起了（比如配对成功后补拉），不重复注册回调
        return Ok(());
    }

    let (event_tx, event_rx) = channel();
    let (conn_tx, conn_rx) = channel();
    let _ = EVENT_TX.set(event_tx);
    let _ = CONN_TX.set(conn_tx);

    let app_for_events = app.clone();
    std::thread::spawn(move || {
        let mut last: Option<(String, bool, Instant)> = None;

        // 长按状态机：确认键（短=center / 长=centerLongPress）、
        // 返回键（短=back / 长=backLongPress，清空语音转写用）。
        // 按住超过 LONG_PRESS 判长按，抬起时若没触发过长按则发短按事件。
        struct LongPress {
            since: Option<Instant>,
            long_fired: bool,
        }
        impl LongPress {
            fn reset(&mut self) {
                *self = LongPress { since: None, long_fired: false };
            }
        }
        let mut lp: [LongPress; 2] = std::array::from_fn(|_| LongPress {
            since: None,
            long_fired: false,
        }); // 0=select, 1=back

        while let Ok(raw) = event_rx.recv() {
            if raw.name == "hidOpenError" || raw.name == "permissionError" {
                // 打开失败 ≠ 未授权。先看真实权限状态，再决定提示文案。
                let access = unsafe { wc_hid_listen_event_access() };
                let detail = if PermissionState::from_raw(access) != PermissionState::Granted {
                    format!("输入监控未授权（IOHID Access={access}），HID Manager 打开失败")
                } else {
                    format!(
                        "HID Manager 打开失败（IOReturn={}）：A2854 未匹配或设备已被占用",
                        raw.value
                    )
                };
                let _ = app_for_events.emit("siri-remote-error", detail.clone());
                // 错误文本交给状态机落库，这里只推信号，避免两处写入
                crate::remote::connection::push(
                    crate::remote::connection::Signal::HidError(detail),
                );
                continue;
            }

            // 原始样本抓取：触摸板报文形态未知，raw / touch / deviceSeen 全量记进
            // diag.json，用真实数据定算法，不给「没生效」留猜的空间。
            // raw 与 touch 都不进按键路径：raw 高频洪泛会淹没去抖与长按计时。
            if raw.name == "raw" || raw.name == "deviceSeen" || raw.name.starts_with("touch") {
                crate::remote::diag::push_raw(
                    &app_for_events,
                    &raw.name,
                    raw.usage_page,
                    raw.usage,
                    raw.value,
                );
                if raw.name.starts_with("touch") {
                    crate::remote::touchpad::push(&raw.name, raw.value);
                }
                continue;
            }

            if raw.name == "select" {
                crate::remote::touchpad::push("touchButton", raw.value);
            }
            let pressed = raw.value != 0;
            let now = Instant::now();

            if let Some((name, was_pressed, at)) = &last {
                if *name == raw.name && *was_pressed == pressed && now.duration_since(*at) < DEDUPE {
                    continue;
                }
            }
            last = Some((raw.name.clone(), pressed, now));

            let event_name = raw.name.clone();
            // debug 构建把每次按键的 usage 打到 stderr：遥控器上有几个键位
            // 各代之间 usage 不同（例如 0x0C/0x60、0x0C/0x04），
            // 让用户按一遍就能把映射表补全，不用来回猜。
            #[cfg(all(debug_assertions, target_os = "macos"))]
            eprintln!(
                "[webcoding] HID page=0x{:02X} usage=0x{:02X} value={} → {}",
                raw.usage_page, raw.usage, raw.value, event_name
            );
            let hid = HidEvent {
                event: event_name.clone(),
                usage_page: raw.usage_page,
                usage: raw.usage,
                value: raw.value,
                pressed,
            };
            {
                let st = app_for_events.state::<AppState>();
                *st.last_hid.lock().unwrap() = Some(hid.clone());
            }
            let _ = app_for_events.emit("siri-remote-input", &hid);

            // 走 Mapping Engine 查当前 Preset 的映射；未配置时引擎会返回提示而不执行
            let fire = |event: &'static str, st: &tauri::State<AppState>| {
                *st.last_event.lock().unwrap() = Some(event.to_string());
                let result = st.engine.lock().unwrap().execute(event);
                crate::remote::diag::push_result(&app_for_events, &result);
                let _ = app_for_events.emit("remote-event", &result);
                result
            };

            let lp_idx = match raw.name.as_str() {
                "select" => Some(0usize),
                "back" => Some(1usize),
                _ => None,
            };

            if let Some(idx) = lp_idx {
                let (short_ev, long_ev) = if idx == 0 {
                    ("center", "centerLongPress")
                } else {
                    ("back", "backLongPress")
                };
                let st = &mut lp[idx];
                if pressed {
                    let started = *st.since.get_or_insert(now);
                    if !st.long_fired && now.duration_since(started) >= LONG_PRESS {
                        st.long_fired = true;
                        let app_state = app_for_events.state::<AppState>();
                        fire(long_ev, &app_state);
                    }
                } else {
                    if !st.long_fired {
                        let app_state = app_for_events.state::<AppState>();
                        fire(short_ev, &app_state);
                    }
                    st.reset();
                }
                continue;
            }

            // 其他按键打断两个长按计时
            for s in lp.iter_mut() {
                s.reset();
            }

            // 只在按下沿触发。抬起沿（value=0）也 fire 的话，一次物理按键 = 两次动作：
            // Cmd+B 会「按一下开、抬手又关」，Cmd+D 会「开始录、抬手又停」——
            // 用户感知就是「连发 + 延迟」。select/back 的两个沿在长按状态机里单独处理。
            if pressed {
                if let Some(event) = to_remote_event(&event_name) {
                    let st = app_for_events.state::<AppState>();
                    fire(event, &st);
                }
            }
        }
    });

    let app_for_conn = app.clone();
    std::thread::spawn(move || {
        while let Ok(count) = conn_rx.recv() {
            let count = count.max(0);
            if count == 0 { crate::remote::touchpad::push("touchReset", 0); }
            // 设备有没有匹配上，是「按键收不到」类问题里最关键的一格读数。
            // 调试构建打到 stderr，省掉一轮「猜 PID」。
            #[cfg(all(debug_assertions, target_os = "macos"))]
            eprintln!(
                "[webcoding] 遥控器 HID 设备数 = {count}，匹配 PID = {}",
                hex_pid(unsafe { wc_siri_remote_matched_pid() })
            );
            let st = app_for_conn.state::<AppState>();
            *st.connected_devices.lock().unwrap() = count as usize;
            let _ = app_for_conn.emit("siri-remote-connection", count);
            crate::remote::connection::push(crate::remote::connection::Signal::HidCount(count));
        }
    });

    // 「还没请求过」时主动走系统授权流程：
    // 不先请求的话，IOHIDManagerOpen 会直接失败，错误原因会被误报成权限问题。
    if PermissionState::from_raw(unsafe { wc_hid_listen_event_access() })
        == PermissionState::Unknown
    {
        unsafe { wc_hid_request_listen_event_access() };
    }

    let rc = unsafe { wc_siri_remote_start(native_event_cb, native_connection_cb) };
    if rc != 0 {
        return Err(format!("启动 A2854 HID 监听失败：{rc}"));
    }

    // 权限看门狗：每 3 秒真实查一次输入监控权限。
    // 用户一旦在系统设置里勾选，自动重开 HID 监听，无需重启 App。
    let app_for_watchdog = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(PERMISSION_RETRY);
        let st = app_for_watchdog.state::<AppState>();
        let access = unsafe { wc_hid_listen_event_access() };
        if PermissionState::from_raw(access) != PermissionState::Granted {
            continue;
        }
        if unsafe { wc_hid_manager_open() } != 0 {
            continue;
        }
        unsafe { wc_siri_remote_restart() };
        let opened = unsafe { wc_hid_manager_open() } != 0;
        {
            let mut guard = st.hid_error.lock().unwrap();
            if opened {
                *guard = None;
            }
        }
        let count = unsafe { wc_siri_remote_connected_count() };
        let _ = app_for_watchdog.emit("siri-remote-connection", count.max(0));
        let _ = app_for_watchdog.emit(
            "siri-remote-ready",
            if opened {
                "输入监控已生效，A2854 HID 监听已自动恢复。若仍无反应，请彻底退出并重启 WebCoding。"
            } else {
                "输入监控已授权，但 HID Manager 仍打开失败，请彻底退出并重启 WebCoding。"
            },
        );
    });

    Ok(())
}

/// 系统诊断状态。权限一律取自 macOS HID / AX API 的真实返回值。
#[cfg(target_os = "macos")]
pub fn system_status(state: &tauri::State<AppState>) -> SystemStatus {
    let count = unsafe { wc_siri_remote_connected_count() };
    let count = count.max(0) as usize;
    SystemStatus {
        input_monitoring: PermissionState::from_raw(unsafe { wc_hid_listen_event_access() }),
        accessibility: if crate::actions::keyboard::accessibility_granted() {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        },
        hid_manager_open: unsafe { wc_hid_manager_open() } != 0,
        input_callback_active: unsafe { wc_hid_callback_active() } != 0,
        a2854_matched: count > 0,
        matched_device_count: count,
        vendor_id: VENDOR_ID.to_string(),
        product_id: {
            let pid = unsafe { wc_siri_remote_matched_pid() };
            (pid > 0).then(|| hex_pid(pid))
        },
        product_id_whitelist: PRODUCT_ID_WHITELIST.iter().map(|s| s.to_string()).collect(),
        app_path: std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "未知".to_string()),
        bundle_id: crate::BUNDLE_ID,
        last_hid_event: state.last_hid.lock().unwrap().clone(),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn system_status(state: &tauri::State<AppState>) -> SystemStatus {
    SystemStatus {
        input_monitoring: PermissionState::Unknown,
        accessibility: PermissionState::Denied,
        hid_manager_open: false,
        input_callback_active: false,
        a2854_matched: false,
        matched_device_count: 0,
        vendor_id: VENDOR_ID.to_string(),
        product_id: None,
        product_id_whitelist: PRODUCT_ID_WHITELIST.iter().map(|s| s.to_string()).collect(),
        app_path: std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "未知".to_string()),
        bundle_id: crate::BUNDLE_ID,
        last_hid_event: state.last_hid.lock().unwrap().clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_hid_names_to_remote_events() {
        assert_eq!(to_remote_event("select"), Some("center"));
        assert_eq!(to_remote_event("back"), Some("back"));
        assert_eq!(to_remote_event("tv"), Some("back"));
        assert_eq!(to_remote_event("siri"), Some("siri"));
        assert_eq!(to_remote_event("playPause"), Some("playPause"));
        assert_eq!(to_remote_event("up"), Some("up"));
    }

    #[test]
    fn ignores_unmapped_hid_usages() {
        assert_eq!(to_remote_event("volumeUp"), None);
        assert_eq!(to_remote_event("volumeDown"), None);
        assert_eq!(to_remote_event("mute"), None);
        assert_eq!(to_remote_event("raw"), None);
        assert_eq!(to_remote_event("permissionError"), None);
    }

    #[test]
    fn long_press_window_is_larger_than_dedupe() {
        assert!(LONG_PRESS > DEDUPE);
    }

    /// 授权后不需要重启 App：看门狗要能在十几秒内重试若干次，但不能太频繁
    #[test]
    fn permission_retry_is_bounded() {
        assert!(PERMISSION_RETRY >= Duration::from_secs(1));
        assert!(PERMISSION_RETRY <= Duration::from_secs(10));
    }

    #[test]
    fn maps_access_codes_to_permission_states() {
        assert_eq!(PermissionState::from_raw(1), PermissionState::Granted);
        assert_eq!(PermissionState::from_raw(2), PermissionState::Denied);
        assert_eq!(PermissionState::from_raw(0), PermissionState::Unknown);
    }
}
