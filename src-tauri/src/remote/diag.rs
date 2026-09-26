//! 诊断快照：把「App 到底看到了什么」落到磁盘。
//!
//! 为什么要有这个文件：排查「遥控器按键没反应」这类问题时，最贵的一步是让用户
//! 截图转述「最近指令」「辅助功能权限」里到底显示了什么。有了这份快照，
//! 直接读文件就能拿到真实读数 —— 有没有收到 HID 事件、派发给了谁、
//! 结果 ok 还是失败、失败原因是什么。
//!
//! 路径：`~/Library/Application Support/com.webcoding.desktop/diag.json`。
//! 周期性整体重写（先写临时文件再 rename，保证读到的不是半截 JSON），
//! `recent` 只留最近 50 条派发结果，体积恒定。

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::mapping::engine::ActionResult;
use crate::AppState;

const CAP: usize = 50;
/// 原始样本环形缓冲：触摸板 ~100 样本/秒，300 条 ≈ 3 秒窗口
const RAW_CAP: usize = 300;
const INTERVAL: Duration = Duration::from_secs(2);

/// 原始 HID 样本抓取（raw / touch*）：报文形态未知时用真实数据说话
pub fn push_raw(app: &AppHandle, name: &str, page: u32, usage: u32, value: i64) {
    if let Some(state) = app.try_state::<AppState>() {
        state.raw_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut list) = state.raw_samples.lock() {
            list.insert(
                0,
                json!({
                    "at": unix_millis(),
                    "name": name,
                    "page": page,
                    "usage": usage,
                    "value": value,
                }),
            );
            list.truncate(RAW_CAP);
        }
    }
}

pub fn raw_snapshot(state: &AppState) -> (u64, Vec<serde_json::Value>) {
    let total = state.raw_total.load(std::sync::atomic::Ordering::Relaxed);
    let samples = state
        .raw_samples
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    (total, samples)
}

/// 派发结果进环形缓冲（最新在前）。HID 与 WebSocket 两条派发路径都要调，
/// 这样「按键到底触发没触发、触发了什么动作」在文件里一目了然。
pub fn push_result(app: &AppHandle, result: &ActionResult) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut list) = state.recent_results.lock() {
            list.insert(0, result.clone());
            list.truncate(CAP);
        }
    }
}

/// 启动周期性快照线程，整个进程只需一次
pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(INTERVAL);
        write_snapshot(&app);
    });
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("diag.json"))
}

fn write_snapshot(app: &AppHandle) {
    let Some(path) = path(app) else {
        return;
    };
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };

    let status = crate::status_snapshot(&state);
    let connection = state.connection_state.lock().ok().map(|g| g.clone());
    let last_hid = state.last_hid.lock().ok().and_then(|g| g.clone());
    let recent = state
        .recent_results
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let (raw_total, raw_samples) = raw_snapshot(state.inner());

    let payload = json!({
        "ts": unix_seconds(),
        "status": &status,
        "connection": connection,
        "lastHid": last_hid,
        "recent": recent,
        "touchpad": crate::remote::touchpad::diagnostics(),
        "rawTotal": raw_total,
        "rawSamples": raw_samples,
    });

    let Ok(text) = serde_json::to_string_pretty(&payload) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
