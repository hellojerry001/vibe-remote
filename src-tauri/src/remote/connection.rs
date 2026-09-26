//! 连接状态机：把「蓝牙配对 / 蓝牙连接 / HID 输入」三层真实读数收敛成一个状态。
//!
//! 关键约定（别把它们混成一个）：
//! - **蓝牙配对成功 ≠ 蓝牙已连接** ≠ **HID 可以读按键**。
//!   配对只是密钥交换完成；连接才是链路通；HID 可读还要求 Input Monitoring 授权。
//! - 状态只由 Rust / Native 层的真实读数推导（蓝牙列表来自 `system_profiler` 子进程，
//!   链路与 HID 来自 IOKit），前端只负责渲染 `ConnectionState`，不做任何推断。

use serde::Serialize;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use tauri::{Emitter, Manager};

use crate::AppState;

/// 十值状态机，UI 的进度条 / 向导步骤完全由它驱动
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteMachine {
    /// 初始态：还没开始
    #[default]
    Idle,
    /// 提示用户进入遥控器配对模式
    Instructions,
    /// 正在搜索附近蓝牙设备
    Scanning,
    /// 发现设备，等待用户点「配对」
    Found,
    /// 应用内配对进行中
    Pairing,
    /// 配对完成，等链路连上
    Paired,
    /// HID 设备已接入，等 Input Monitoring 生效
    Connecting,
    /// 链路已通
    Connected,
    /// 三层全部就绪（配对 + 连接 + HID 输入）
    HidReady,
    /// 失败，需要重试
    Failed,
}

impl RemoteMachine {
    /// 向导第 N / 3 步（权限向导用）
    pub fn step(self) -> Option<u8> {
        Some(match self {
            RemoteMachine::Idle | RemoteMachine::Instructions => 1,
            RemoteMachine::Scanning | RemoteMachine::Found | RemoteMachine::Pairing => 2,
            RemoteMachine::Paired | RemoteMachine::Connecting | RemoteMachine::Connected => 3,
            RemoteMachine::HidReady => 4,
            RemoteMachine::Failed => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            RemoteMachine::Idle => "待准备好",
            RemoteMachine::Instructions => "让遥控器进入配对模式",
            RemoteMachine::Scanning => "搜索中",
            RemoteMachine::Found => "发现遥控器",
            RemoteMachine::Pairing => "配对中",
            RemoteMachine::Paired => "已配对",
            RemoteMachine::Connecting => "连接中",
            RemoteMachine::Connected => "已连接",
            RemoteMachine::HidReady => "可用（HID 输入已就绪）",
            RemoteMachine::Failed => "连接失败",
        }
    }
}

/// 单层状态：独立表达这一层到底通没通
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerLevel {
    /// 这层根本没参与
    Off,
    /// 参与了但还没打通
    Pending,
    /// 已打通
    Working,
    /// 卡住了，且原因是明确的
    Error,
}

/// 驱动状态机的信号。三个来源都往这个通道里推。
#[derive(Debug, Clone)]
pub enum Signal {
    /// 用户点了「开始搜索」
    ScanStarted,
    /// 扫描结束，payload = 是否搜到过设备
    ScanFinished(bool),
    /// 发现设备（is_siri_remote 为 true 时可直接高亮）
    DeviceFound { is_siri_remote: bool },
    /// 配对阶段变化。
    /// 进程内无法发起配对，所以这里收到的是「用户在系统蓝牙里配对进展到哪一步」的旁证：
    /// Connecting = A2854 已在蓝牙列表里但链路还没通，Finished = 链路通了。
    Pairing(PairPhase),
    /// HID 侧报告当前接入设备数
    HidCount(i32),
    /// HID 侧报错
    HidError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairPhase {
    /// A2854 已在系统蓝牙列表里，但链路还没通
    Connecting,
    /// 链路已经通了
    Finished,
    /// 配对着呢，但当前读数看不出结论
    Failed,
}

/// 一次真实读数：全部来自 Native 侧的真实查询，没有任何「猜」
#[derive(Debug, Clone, Copy)]
pub struct Readings {
    /// 系统蓝牙里是否已见过 A2854。
    /// 注意它不是 `IOBluetoothDevice.isPaired` —— 应用内无法配对（进程会被 TCC abort），
    /// 只能靠 system_profiler 的读数旁证「它进过配对流程」。
    pub paired: bool,
    /// 当前接入的 A2854 数量（IOHID 侧计数）
    pub device_count: i32,
    /// IOHIDManager 是否已打开
    pub hid_manager_open: bool,
    /// 输入回调是否真的在跑
    pub input_callback: bool,
    /// 输入监控权限是否已授权
    pub input_monitoring: bool,
}

impl Readings {
    /// 只用来做「配对」判据：配对成功即蓝牙层可用
    fn bluetooth_layer(self) -> (LayerLevel, &'static str, String) {
        if self.paired {
            (LayerLevel::Working, "Bluetooth", "已配对".to_string())
        } else {
            (
                LayerLevel::Pending,
                "Bluetooth",
                "未配对，需要配对一次".to_string(),
            )
        }
    }

    fn connection_layer(self) -> (LayerLevel, &'static str, String) {
        if self.device_count > 0 {
            (
                LayerLevel::Working,
                "Connection",
                format!("已连接（{} 台）", self.device_count),
            )
        } else if self.paired {
            (
                LayerLevel::Pending,
                "Connection",
                "已配对，等待链路连上".to_string(),
            )
        } else {
            (LayerLevel::Off, "Connection", "未连接".to_string())
        }
    }

    fn hid_layer(self) -> (LayerLevel, &'static str, String) {
        if self.device_count <= 0 {
            // 设备都没匹配上时，权限不是第一矛盾 —— 别把人往权限页带
            return (
                LayerLevel::Off,
                "HID Input",
                "未匹配到遥控器".to_string(),
            );
        }
        if !self.input_monitoring {
            return (
                LayerLevel::Error,
                "HID Input",
                "输入监控未授权".to_string(),
            );
        }
        if self.input_callback && self.hid_manager_open {
            return (LayerLevel::Working, "HID Input", "按键可读取".to_string());
        }
        (
            LayerLevel::Pending,
            "HID Input",
            "等待遥控器上报按键".to_string(),
        )
    }
}

impl Default for Readings {
    fn default() -> Self {
        Self {
            paired: false,
            device_count: 0,
            hid_manager_open: false,
            input_callback: false,
            input_monitoring: false,
        }
    }
}

/// 前端直接渲染的快照：十值机 + 三层独立读数
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionState {
    pub machine: RemoteMachine,
    pub machine_label: &'static str,
    pub step: Option<u8>,
    pub bluetooth: LayerLevel,
    pub bluetooth_detail: String,
    pub connection: LayerLevel,
    pub connection_detail: String,
    pub hid: LayerLevel,
    pub hid_detail: String,
    pub a2854_matched: bool,
    pub error: Option<String>,
}

impl ConnectionState {
    pub fn build(machine: RemoteMachine, r: Readings, error: Option<String>) -> Self {
        let (bt_level, _, bt_detail) = r.bluetooth_layer();
        let (conn_level, _, conn_detail) = r.connection_layer();
        let (hid_level, _, hid_detail) = r.hid_layer();
        Self {
            machine,
            machine_label: machine.label(),
            step: machine.step(),
            bluetooth: bt_level,
            bluetooth_detail: bt_detail,
            connection: conn_level,
            connection_detail: conn_detail,
            hid: hid_level,
            hid_detail: hid_detail,
            a2854_matched: r.device_count > 0,
            error,
        }
    }
}

/// 状态机推进：一条信号只可能被少数几个状态接受
pub fn next_machine(current: RemoteMachine, signal: &Signal) -> RemoteMachine {
    if current == RemoteMachine::Failed {
        // 失败之后只有「重新搜索」能脱离
        return match signal {
            Signal::ScanStarted => RemoteMachine::Scanning,
            _ => RemoteMachine::Failed,
        };
    }

    match signal {
        Signal::ScanStarted => match current {
            RemoteMachine::Idle | RemoteMachine::Instructions | RemoteMachine::Paired => {
                RemoteMachine::Scanning
            }
            _ => current,
        },
        // 搜到了就停在 Found 等用户选设备；空气就退回 Instructions 让他再试一次
        Signal::ScanFinished(found) => match current {
            RemoteMachine::Scanning => {
                if *found {
                    RemoteMachine::Found
                } else {
                    RemoteMachine::Instructions
                }
            }
            _ => current,
        },
        Signal::DeviceFound { is_siri_remote } => match current {
            RemoteMachine::Scanning | RemoteMachine::Instructions | RemoteMachine::Idle => {
                // 只有 Siri Remote 才推进到「发现遥控器」；别的 BLE 设备只是陪跑
                if *is_siri_remote {
                    RemoteMachine::Found
                } else {
                    RemoteMachine::Scanning
                }
            }
            _ => current,
        },
        Signal::Pairing(phase) => match (current, phase) {
            (_, PairPhase::Finished) => RemoteMachine::Paired,
            (_, PairPhase::Failed) => RemoteMachine::Failed,
            (RemoteMachine::Scanning, _)
            | (RemoteMachine::Found, _)
            | (RemoteMachine::Pairing, _)
            | (RemoteMachine::Paired, _) => RemoteMachine::Pairing,
            (_, _) => current,
        },
        // HID 侧只做连线性反馈，真正的跃迁交给 finalize
        Signal::HidCount(_) | Signal::HidError(_) => current,
    }
}

/// 按最新读数收敛链路阶段：Paired → Connecting → Connected → HidReady。
///
/// 唯一的降档路径是设备掉线（device_count 归零），保证「已就绪」不会在遥控器
/// 已经离开时继续显示。
pub fn finalize(machine: RemoteMachine, r: Readings) -> RemoteMachine {
    let (conn, hid) = (r.connection_layer().0, r.hid_layer().0);
    match machine {
        RemoteMachine::Paired => match (conn, hid) {
            (LayerLevel::Working, LayerLevel::Working) => RemoteMachine::HidReady,
            (LayerLevel::Working, _) => RemoteMachine::Connected,
            _ => RemoteMachine::Connecting,
        },
        RemoteMachine::Connecting | RemoteMachine::Connected | RemoteMachine::HidReady => {
            match (conn, hid) {
                (LayerLevel::Working, LayerLevel::Working) => RemoteMachine::HidReady,
                (LayerLevel::Working, _) => RemoteMachine::Connected,
                // 链路断了 → 退回「已配对，等待连上」
                _ if r.device_count == 0 && r.paired => RemoteMachine::Paired,
                _ => machine,
            }
        }
        _ => machine,
    }
}

static SIGNAL_TX: OnceLock<Sender<Signal>> = OnceLock::new();

/// 供 bluetooth / hid 等模块推信号
pub fn push(signal: Signal) {
    if let Some(tx) = SIGNAL_TX.get() {
        let _ = tx.send(signal);
    }
}

/// 读取一次真实读数
#[cfg(target_os = "macos")]
fn read(state: &tauri::State<AppState>) -> Readings {
    unsafe {
        Readings {
            paired: state
                .paired_device
                .lock()
                .map(|g| g.is_some())
                .unwrap_or(false),
            device_count: wc_siri_remote_connected_count().max(0),
            hid_manager_open: wc_hid_manager_open() != 0,
            input_callback: wc_hid_callback_active() != 0,
            input_monitoring: wc_hid_listen_event_access() == 1,
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn read(state: &tauri::State<AppState>) -> Readings {
    let _ = state;
    Readings::default()
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn wc_siri_remote_connected_count() -> i32;
    fn wc_hid_manager_open() -> i32;
    fn wc_hid_callback_active() -> i32;
    fn wc_hid_listen_event_access() -> i32;
}

fn republish(app: &tauri::AppHandle, machine: RemoteMachine, state: &tauri::State<AppState>) {
    let r = read(state);
    let machine = finalize(machine, r);
    let error = state.hid_error.lock().ok().and_then(|g| g.clone());
    let snapshot = ConnectionState::build(machine, r, error);
    if let Ok(mut guard) = state.connection_state.lock() {
        *guard = snapshot.clone();
    }
    let _ = app.emit("remote-connection-state", &snapshot);
}

/// 启动状态机消费线程。整个进程只需一次。
pub fn start(app: &tauri::AppHandle) {
    if SIGNAL_TX.get().is_some() {
        return;
    }
    let (tx, rx) = channel::<Signal>();
    let _ = SIGNAL_TX.set(tx);

    let handle = app.clone();
    std::thread::spawn(move || {
        let state = handle.state::<AppState>();
        machine_loop(&handle, &state, rx);
    });
}

fn machine_loop(
    app: &tauri::AppHandle,
    state: &tauri::State<AppState>,
    rx: Receiver<Signal>,
) {
    let mut machine = RemoteMachine::default();
    let mut last_count = i32::MIN;
    // 初值也要发一份，UI 一进来就有正确的「未连接」三行
    republish(app, machine, state);

    while let Ok(signal) = rx.recv() {
        // HID 设备数是链路层唯一可信的触发源：变了就要按新读数重算快照
        let hid_count_changed = matches!(&signal, Signal::HidCount(n) if *n != last_count);
        if let Signal::HidCount(n) = &signal {
            last_count = *n;
        }

        if let Signal::Pairing(phase) = &signal {
            if *phase == PairPhase::Failed {
                // 配对失败时顺手清掉上一次配对成功的残留
                if let Ok(mut g) = state.paired_device.lock() {
                    *g = None;
                }
            }
        }
        // HID 报错文本由状态机统一落库，AppState.hid_error 只有一个写入方
        if let Signal::HidError(detail) = &signal {
            if let Ok(mut guard) = state.hid_error.lock() {
                *guard = Some(detail.clone());
            }
        }
        let next = next_machine(machine, &signal);
        let changed = next != machine;
        machine = next;

        // 状态机变了、或者只是读数变了（HID 计数 / 错误），都要重新广播快照。
        // 掉线降档交给 republish 里的 finalize，避免两处改状态。
        if changed || hid_count_changed || matches!(signal, Signal::HidError(_)) {
            republish(app, machine, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readings(paired: bool, count: i32, callback: bool) -> Readings {
        Readings {
            paired,
            device_count: count,
            hid_manager_open: callback,
            input_callback: callback,
            input_monitoring: true,
        }
    }

    #[test]
    fn walks_the_happy_path() {
        let mut m = RemoteMachine::default();
        m = next_machine(m, &Signal::ScanStarted);
        assert_eq!(m, RemoteMachine::Scanning);
        m = next_machine(m, &Signal::DeviceFound { is_siri_remote: true });
        assert_eq!(m, RemoteMachine::Found);
        m = next_machine(m, &Signal::Pairing(PairPhase::Connecting));
        assert_eq!(m, RemoteMachine::Pairing);
        m = next_machine(m, &Signal::Pairing(PairPhase::Finished));
        assert_eq!(m, RemoteMachine::Paired);
        // 还没接上设备 → finalize 停在 Connecting
        m = finalize(m, readings(true, 0, false));
        assert_eq!(m, RemoteMachine::Connecting);
        // 设备接入，finalize 按真实读数推进到就绪
        m = finalize(m, readings(true, 1, true));
        assert_eq!(m, RemoteMachine::HidReady);
        // 设备掉线：收敛回「已配对」，不能继续显示可用
        m = finalize(m, readings(true, 0, true));
        assert_eq!(m, RemoteMachine::Paired);
    }

    #[test]
    fn pairing_failure_is_reachable_and_recoverable() {
        let m = next_machine(RemoteMachine::Found, &Signal::Pairing(PairPhase::Failed));
        assert_eq!(m, RemoteMachine::Failed);
        // 失败后只有重新搜索能出来
        assert_eq!(
            next_machine(m, &Signal::DeviceFound { is_siri_remote: true }),
            RemoteMachine::Failed
        );
        assert_eq!(
            next_machine(m, &Signal::ScanStarted),
            RemoteMachine::Scanning
        );
    }

    #[test]
    fn empty_scan_returns_to_instructions() {
        let mut m = RemoteMachine::default();
        m = next_machine(m, &Signal::ScanStarted);
        m = next_machine(m, &Signal::ScanFinished(false));
        assert_eq!(m, RemoteMachine::Instructions);
    }

    #[test]
    fn three_layers_are_reported_independently() {
        // 配对了、也连上了，但输入监控没授权 → HID 这一层单独报错
        let r = Readings {
            paired: true,
            device_count: 1,
            hid_manager_open: false,
            input_callback: false,
            input_monitoring: false,
        };
        let snap = ConnectionState::build(RemoteMachine::Paired, r, None);
        assert_eq!(snap.bluetooth, LayerLevel::Working);
        assert_eq!(snap.connection, LayerLevel::Working);
        assert_eq!(snap.hid, LayerLevel::Error);
        assert_eq!(snap.hid_detail, "输入监控未授权");

        // 完全没配对 → 连接层还没参与
        let r = Readings::default();
        let snap = ConnectionState::build(RemoteMachine::Idle, r, None);
        assert_eq!(snap.bluetooth, LayerLevel::Pending);
        assert_eq!(snap.connection, LayerLevel::Off);
        assert_eq!(snap.hid, LayerLevel::Off);
    }

    #[test]
    fn hid_ready_is_the_only_terminal_success() {
        let m = finalize(RemoteMachine::Paired, readings(true, 1, true));
        assert_eq!(m, RemoteMachine::HidReady);
        assert_eq!(m.step(), Some(4));
    }
}
