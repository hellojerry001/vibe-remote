//! A2854 蓝牙发现与配对状态推进。
//!
//! ⚠️ 这里刻意**不在进程内调用任何蓝牙 API**，别改回去：
//! macOS 13+ 上，只要进程碰蓝牙（IOBluetooth.framework 的
//! `IOBluetoothDeviceInquiry` / `IOBluetoothDevicePair`，或 CoreBluetooth 的
//! `CBCentralManager`），TCC 就会走
//! `__TCC_CRASHING_DUE_TO_PRIVACY_VIOLATION__` **直接 abort 整个进程**（SIGABRT），
//! 不是返回错误码。实测过的无效尝试：往 Info.plist 加
//! `NSBluetoothAlwaysUsageDescription` / `BluetoothUsageDescription` /
//! `NSBluetoothPeripheralUsageDescription`、ad-hoc 签名、放到
//! `/Applications` 下、起一个完整 `NSApplication` + 窗口 —— 全都照崩。
//! 表现就是「点一下开始搜索，App 直接蹦掉」。
//!
//! 所以发现设备改成读 `system_profiler SPBluetoothDataType` 的输出：
//! 它是独立进程、不受本进程 TCC 判定约束，能拿到 名称 / 地址 / VID / PID / 是否连接。
//! 由此产生的两个能力缺口是**系统限制**，不是功能没做：
//!   · 拿不到 RSSI（该命令不输出信号强度）
//!   · 不能发起应用内配对 → 配对只能在系统蓝牙设置里完成，本模块只负责把
//!     「配对中 → 已配对」这个跃迁如实推给状态机。

use serde::Serialize;
use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::remote::connection::{self, PairPhase, Signal};
use crate::AppState;

/// 轮询间隔：遥控器进入配对模式后大约 1.5s 内会被系统蓝牙记录
const POLL_INTERVAL: Duration = Duration::from_millis(1500);
/// 一轮扫描的总时长上限，跟 UI 上写的「约 10 秒」保持一致
const SCAN_BUDGET: Duration = Duration::from_secs(10);
/// Apple 的 VID
const A2854_VENDOR_ID: u32 = 0x004C;
/// Apple TV 遥控器家族的 PID 白名单，与 native/siri_remote_bridge.c 保持一致。
///
/// ⚠️ 别只认 0x0315：A2854（USB-C 第三代）实测是 **0x0314**，A2540（第二代）才是 0x0315。
/// 更要命的是它在蓝牙列表里的名字是随机串（本机实测 `C08HP16M17YC`，根本不是
/// "Siri Remote"），**纯靠名字判断一定会漏**，VID+PID 才是可靠判据。
const A2854_PRODUCT_IDS: [u32; 4] = [0x0266, 0x026D, 0x0314, 0x0315];

/// 系统蓝牙里的一台设备
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BtDevice {
    pub address: String,
    pub name: String,
    /// 出现在「已连接」分组里；否则是「未连接」（已配对过但链路断开，或刚进入配对模式）
    pub connected: bool,
    pub is_siri_remote: bool,
    pub vendor_id: Option<u32>,
    pub product_id: Option<u32>,
}

/// 一次 `system_profiler` 的解析结果
#[derive(Debug, Default)]
pub struct ScanReport {
    pub devices: Vec<BtDevice>,
    /// 蓝牙开关状态；`true` 表示没读到「State: Off」，即控制器是开着的
    pub controller_on: bool,
}

fn looks_like_siri_remote(name: &str, vendor_id: Option<u32>, product_id: Option<u32>) -> bool {
    // VID + PID 是主判据：遥控器的蓝牙名可能是一串随机字符，名字匹配不上很正常
    if vendor_id == Some(A2854_VENDOR_ID) {
        if let Some(pid) = product_id {
            if A2854_PRODUCT_IDS.contains(&pid) {
                return true;
            }
        }
    }
    // 名字兜底：个别系统/重命名过的情况下还是会带 siri 字样
    let n = name.to_ascii_lowercase();
    n.contains("siri remote") || n.contains("siriremote")
}

/// 解析 `system_profiler SPBluetoothDataType` 的文本输出。
///
/// 缩进层级（实测）：
/// ```text
/// Bluetooth:                    0
/// Bluetooth Controller:         6
///     State: On                10   ← 控制器的字段，不是设备
/// Connected:                    6
///     Siri Remote:            10   ← 设备名
///         Address: xx:..       14   ← 设备字段
///         Vendor ID: 0x004C    14
///         Product ID: 0x0315   14
/// ```
/// 关键在于 `Connected:` / `Not Connected:` 这两个分组头决定了设备属于哪一组。
pub fn parse_system_profiler(output: &str) -> ScanReport {
    let mut report = ScanReport {
        devices: Vec::new(),
        controller_on: true,
    };
    // 当前所处的分组：`Connected` 或 `Not Connected`；`None` 表示不在设备列表里
    let mut group: Option<bool> = None;
    let mut current: Option<BtDevice> = None;

    for line in output.lines() {
        let raw = line.trim_end();
        if raw.trim().is_empty() {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        let text = raw.trim();

        if indent == 0 {
            // 顶层只有 `Bluetooth:`，之后就是各个分组
            current = None;
            continue;
        }

        if indent <= 6 {
            let is_connected = text.eq_ignore_ascii_case("Connected:");
            if is_connected || text.eq_ignore_ascii_case("Not Connected:") {
                // 分组头：先把上一个设备收掉，再用它决定这一组的 connected 值
                if let Some(device) = current.take() {
                    if !device.address.is_empty() {
                        report.devices.push(device);
                    }
                }
                group = Some(is_connected);
            } else {
                // 控制器有自己的字段（State / Address / Chipset），算设备会污染列表
                current = None;
                group = None;
            }
            continue;
        }

        if indent == 10 && group.is_some() {
            // 设备名行：形如 `Siri Remote:`
            if let Some(device) = current.take() {
                if !device.address.is_empty() {
                    report.devices.push(device);
                }
            }
            let name = text.strip_suffix(':').unwrap_or(text).trim().to_string();
            current = Some(BtDevice {
                address: String::new(),
                name,
                connected: group.unwrap_or(false),
                is_siri_remote: false,
                vendor_id: None,
                product_id: None,
            });
            continue;
        }

        // 缩进 10 起就是字段：设备的字段在 14，控制器自己的字段也在 10
        if indent >= 10 {
            let Some((key, value)) = split_field(text) else {
                continue;
            };
            match current.as_mut() {
                Some(device) => match key {
                    "Address" => device.address = value.to_string(),
                    "Vendor ID" => device.vendor_id = parse_hex(value),
                    "Product ID" => device.product_id = parse_hex(value),
                    _ => {}
                },
                // 没有在设备上下文里，就只关心控制器的开关状态
                None => {
                    if key == "State" && value.eq_ignore_ascii_case("Off") {
                        report.controller_on = false;
                    }
                }
            }
        }
    }
    if let Some(device) = current {
        if !device.address.is_empty() {
            report.devices.push(device);
        }
    }

    for device in report.devices.iter_mut() {
        device.is_siri_remote = looks_like_siri_remote(&device.name, device.vendor_id, device.product_id);
    }
    report
}

/// `Key: Value`，只认第一个 `": "`
fn split_field(text: &str) -> Option<(&str, &str)> {
    let at = text.find(": ")?;
    Some((text[..at].trim(), text[at + 2..].trim()))
}

fn parse_hex(value: &str) -> Option<u32> {
    // 控制器那行会写成 `0x004C (Apple)`，只取第一段
    let text = value.split_whitespace().next()?;
    let body = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))?;
    u32::from_str_radix(body, 16).ok()
}

/// 跑一次 `system_profiler`；失败不抛，交给上层退化成「没搜到」
fn run_profiler() -> Result<ScanReport, String> {
    let output = Command::new("system_profiler")
        .args(["SPBluetoothDataType"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("无法启动 system_profiler：{e}"))?;
    Ok(parse_system_profiler(&String::from_utf8_lossy(&output.stdout)))
}

static SCANNING: AtomicBool = AtomicBool::new(false);
static DEVICE_TX: OnceLock<Sender<BtDevice>> = OnceLock::new();

/// 惰性起一个消费线程，扫描结果从它这里转成 Tauri 事件
fn ensure_thread(app: &AppHandle) {
    let _ = DEVICE_TX.get_or_init(|| {
        let (tx, rx) = channel();
        let handle = app.clone();
        std::thread::spawn(move || consume_devices(&handle, rx));
        tx
    });
}

fn consume_devices(app: &AppHandle, rx: Receiver<BtDevice>) {
    while let Ok(device) = rx.recv() {
        report_device(app, device);
    }
}

/// 由消费线程调用：落库 → 广播 → 推信号
fn report_device(app: &AppHandle, device: BtDevice) {
    {
        let st = app.state::<AppState>();
        let mut list = st.bt_devices.lock().unwrap();
        if !list.iter().any(|d| d.address == device.address) {
            list.push(device.clone());
        }
    }
    let _ = app.emit("bt-device-found", &device);
    if device.is_siri_remote {
        let st = app.state::<AppState>();
        *st.paired_device.lock().unwrap() = Some(device.name.clone());
    }
    connection::push(Signal::DeviceFound {
        is_siri_remote: device.is_siri_remote,
    });
}

/// 开始扫描附近蓝牙设备（幂等：已经在扫就忽略）
pub fn start_scan(app: &AppHandle) {
    if SCANNING.swap(true, Ordering::SeqCst) {
        return;
    }
    ensure_thread(app);
    let handle = app.clone();
    std::thread::spawn(move || scan_loop(&handle));
}

/// 停止扫描；扫描线程最多在一个轮询周期后退出
pub fn stop_scan() {
    SCANNING.store(false, Ordering::SeqCst);
}

fn scan_loop(app: &AppHandle) {
    let deadline = Instant::now() + SCAN_BUDGET;
    let mut seen: HashSet<String> = HashSet::new();
    let mut siri_seen_before = false;
    let mut any = false;

    while Instant::now() < deadline {
        match run_profiler() {
            Ok(report) => {
                if !report.controller_on {
                    let _ = app.emit("bt-scan-note", "这台 Mac 的蓝牙是关闭的，先打开它再搜索。");
                    break;
                }
                for device in report.devices {
                    if !seen.insert(device.address.clone()) {
                        continue;
                    }
                    any = true;
                    // 先拷出判定要用到的两个字段，再把它交给消费线程
                    let (is_siri, linked) = (device.is_siri_remote, device.connected);
                    if let Some(tx) = DEVICE_TX.get() {
                        let _ = tx.send(device);
                    }
                    // 第二次及以后还看得到 A2854，说明配对流程在系统蓝牙里推进着：
                    // 链路还挂着就报「配对中」，链路通了就报「已配对」。
                    if is_siri && siri_seen_before {
                        connection::push(Signal::Pairing(if linked {
                            PairPhase::Finished
                        } else {
                            PairPhase::Connecting
                        }));
                    }
                    siri_seen_before |= is_siri;
                }
            }
            Err(e) => {
                let _ = app.emit("bt-scan-note", e);
                break;
            }
        }

        if !SCANNING.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    SCANNING.store(false, Ordering::SeqCst);
    connection::push(Signal::ScanFinished(any));
    let _ = app.emit("bt-scan-finished", ());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实样本（本机 system_profiler 输出的结构，只保留相关设备）。
    /// 缩进层级必须照抄：分组头 6、设备名 10、设备字段 14、控制器字段 10。
    const SAMPLE: &str = "\
Bluetooth:

      Bluetooth Controller:
          Address: 5C:E9:1E:B8:08:AE
          State: On
          Chipset: BCM_4388

      Connected:
          C08HP16M17YC:
              Address: 58:0A:D4:B7:27:FA
              Vendor ID: 0x004C
              Product ID: 0x0314
              Battery Level: 100%

      Not Connected:
          ACTON III:
              Address: 78:5E:A2:44:B7:72
              Vendor ID: 0x0094
              Product ID: 0x0004
              Minor Type: Speaker
          Logitech Pebble:
              Address: D7:E5:6E:96:EE:7C
              Minor Type: Mouse
          Siri Remote:
              Address: F7:2E:8B:11:22:33
              Vendor ID: 0x004C
              Product ID: 0x0315
";

    fn with(sample: &str) -> Vec<BtDevice> {
        parse_system_profiler(sample).devices
    }

    #[test]
    fn parses_devices_with_address_and_ids() {
        let devices = with(SAMPLE);
        // 控制器字段不能被当成设备
        assert_eq!(devices.len(), 4, "实际：{devices:?}");
        assert_eq!(devices[0].name, "C08HP16M17YC");
        assert_eq!(devices[0].address, "58:0A:D4:B7:27:FA");
        assert_eq!(devices[0].vendor_id, Some(0x004C));
        assert_eq!(devices[0].product_id, Some(0x0314));

        let speaker = &devices[1];
        assert_eq!(speaker.name, "ACTON III");
        assert_eq!(speaker.vendor_id, Some(0x0094));
        assert!(!speaker.connected);

        // 没有 Address 的设备直接丢掉，别让 UI 出现空行
        assert!(with(SAMPLE)
            .iter()
            .all(|d| !d.address.is_empty()));
    }

    /// 真机实测：A2854 在蓝牙列表里的名字是 `C08HP16M17YC`（随机串，不含 "siri"），
    /// PID 是 0x0314。所以判据必须是 VID+PID，不能是名字，也不能只认 0x0315。
    #[test]
    fn identifies_the_siri_remote_by_vid_pid_not_by_name() {
        let devices = with(SAMPLE);

        let real = &devices[0];
        assert_eq!(real.name, "C08HP16M17YC");
        assert!(
            real.is_siri_remote,
            "0x004C:0x0314 就是本机连着的 A2854，名字不含 siri 也必须认出来"
        );

        // 第二代（0x0315）同样在白名单里
        let second_gen = devices.iter().find(|d| d.name == "Siri Remote").unwrap();
        assert!(second_gen.is_siri_remote);
        assert_eq!(second_gen.product_id, Some(0x0315));

        // 非 Apple 设备
        assert!(!devices[1].is_siri_remote);

        // Apple VID 但 PID 不在白名单 → 不能算遥控器
        assert!(
            !looks_like_siri_remote("Magic Keyboard", Some(0x004C), Some(0x0265)),
            "只靠 VID 猜会误伤其它 Apple 配件"
        );
        // 名字带 siri 但没有 VID 信息 → 兜底认出来
        assert!(looks_like_siri_remote("Siri Remote", None, None));
    }

    #[test]
    fn detects_a_switched_off_controller() {
        let off = "Bluetooth:\n\n      Bluetooth Controller:\n          State: Off\n";
        assert!(!parse_system_profiler(off).controller_on);
        assert!(parse_system_profiler(SAMPLE).controller_on);
    }

    #[test]
    fn matches_by_name_case_insensitively() {
        assert!(looks_like_siri_remote("siriremote", None, None));
        assert!(looks_like_siri_remote("SIRI REMOTE A2854", None, None));
        assert!(looks_like_siri_remote("Siri Remote", None, None));
        assert!(!looks_like_siri_remote("WH-1000XM3", Some(0x054C), Some(0x0CD3)));
    }

    /// 端到端：真跑一次 `system_profiler`。
    /// 这条链路就是「开始搜索」实际走的路，能防住「改了解析但没 discovers 到任何设备」。
    #[test]
    fn reads_devices_from_the_real_system() {
        if cfg!(not(target_os = "macos")) {
            return;
        }
        let report = run_profiler().expect("system_profiler 应该能执行");
        assert!(
            !report.devices.is_empty(),
            "解析不到任何设备 —— system_profiler 的输出格式可能变了"
        );
        assert!(
            report.devices.iter().all(|d| !d.address.is_empty()),
            "有设备缺 Address，解析层级有问题：{report:?}"
        );
    }

    #[test]
    fn parses_hex_ids() {
        assert_eq!(parse_hex("0x004C"), Some(0x004C));
        assert_eq!(parse_hex("0X0315"), Some(0x0315));
        assert_eq!(parse_hex("Apple"), None);
        assert_eq!(parse_hex(""), None);
    }
}
