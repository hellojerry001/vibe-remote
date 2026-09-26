use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::actions::{accessibility, applescript, keyboard, shell};

/// 遥控事件。tvOS 端走 WebSocket，A2854 蓝牙直连经 `remote::hid` 适配到同样的名字，
/// 两个输入源在这里彻底汇合，下游只认这些事件名。
pub const REMOTE_EVENTS: [&str; 10] = [
    "up",
    "down",
    "left",
    "right",
    "center",
    "back",
    "backLongPress",
    "playPause",
    "centerLongPress",
    "siri",
];

/// 内置动作（kind = "action" 时 value 的合法取值）
pub const BUILTIN_ACTIONS: [&str; 7] = [
    "approve",
    "reject",
    "approve_all",
    "clear_input",
    "continue",
    "send",
    "voice_input",
];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MappingKind {
    /// 组合快捷键，如 "Cmd+Enter"
    Shortcut,
    /// 单个按键名，如 "escape"、"up"
    Key,
    /// 内置语义动作，如 "approve"
    Action,
    /// 任意 AppleScript 脚本
    Applescript,
    /// 任意 shell 命令（/bin/zsh -c）
    Shell,
    /// 宏序列，value 形如 "Cmd+D|wait:1500|Cmd+Down|Cmd+Right"：
    /// 步骤用 | 分隔，每步是 wait:N（毫秒）或快捷键/按键名
    Macro,
}

impl MappingKind {
    pub fn parse(s: &str) -> Option<MappingKind> {
        match s {
            "shortcut" => Some(MappingKind::Shortcut),
            "key" => Some(MappingKind::Key),
            "action" => Some(MappingKind::Action),
            "applescript" => Some(MappingKind::Applescript),
            "shell" => Some(MappingKind::Shell),
            "macro" => Some(MappingKind::Macro),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MappingKind::Shortcut => "shortcut",
            MappingKind::Key => "key",
            MappingKind::Action => "action",
            MappingKind::Applescript => "applescript",
            MappingKind::Shell => "shell",
            MappingKind::Macro => "macro",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mapping {
    pub kind: MappingKind,
    #[serde(default)]
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub mappings: BTreeMap<String, Mapping>,
    /// 固定把按键发给哪个 App（App 名或 bundle id，如 "WorkBuddy" /
    /// "com.tencent.workbuddy.mac"）。留空 = 发给「当前前台 App」。
    ///
    /// 这是「遥控器控制 WorkBuddy」的关键：不依赖你正好盯着哪个窗口 ——
    /// 你在看 Web Coding 的设置页时，按键也不会落错地方。
    #[serde(default)]
    pub target_app: Option<String>,
}

impl Preset {
    fn with_mappings(mappings: BTreeMap<String, Mapping>) -> Self {
        Preset {
            mappings,
            target_app: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub preset: String,
    pub presets: BTreeMap<String, Preset>,
    /// 触摸板 → 鼠标。老配置缺这个字段时 serde default 补全（默认开）。
    #[serde(default)]
    pub touchpad: TouchpadConfig,
}

/// 触摸板管线参数，运行时即时生效。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TouchpadConfig {
    pub enabled: bool,
    /// 光标增益：HID 原生位移 × gain = 屏幕像素
    pub gain: f64,
    /// 轻点触摸板 = 左键点击
    pub tap_click: bool,
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        TouchpadConfig { enabled: true, gain: 10.0, tap_click: true }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub event: String,
    pub kind: String,
    pub value: String,
    pub ok: bool,
    pub detail: String,
}

impl ActionResult {
    fn new(event: &str, kind: &str, value: &str) -> Self {
        ActionResult {
            event: event.to_string(),
            kind: kind.to_string(),
            value: value.to_string(),
            ok: false,
            detail: String::new(),
        }
    }
}

pub struct Engine {
    pub config: Config,
    path: PathBuf,
}

impl Engine {
    pub fn load(path: PathBuf) -> Engine {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str::<Config>(&text) {
                let engine = Engine {
                    config: normalize(config),
                    path,
                };
                // 归一化可能补了新事件槽位，立刻落盘，保证 UI 显示的和磁盘一致
                engine.save();
                return engine;
            }
        }
        let engine = Engine {
            config: default_config(),
            path: path.clone(),
        };
        engine.save();
        engine
    }

    pub fn set_touchpad(&mut self, config: TouchpadConfig) -> Result<(), String> {
        let previous = self.config.touchpad.clone();
        self.config.touchpad = config;
        let result = serde_json::to_string_pretty(&self.config).map_err(|e| e.to_string())
            .and_then(|text| std::fs::write(&self.path, text).map_err(|e| e.to_string()));
        if result.is_err() { self.config.touchpad = previous; }
        result
    }

    pub fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.config) {
            let _ = std::fs::write(&self.path, text);
        }
    }

    pub fn mapping_for(&self, event: &str) -> Option<Mapping> {
        self.config
            .presets
            .get(&self.config.preset)?
            .mappings
            .get(event)
            .cloned()
    }

    pub fn set_mapping(&mut self, preset: &str, event: &str, mapping: Mapping) -> Result<(), String> {
        let target = self
            .config
            .presets
            .get_mut(preset)
            .ok_or_else(|| format!("Preset「{preset}」不存在"))?;
        if !REMOTE_EVENTS.contains(&event) {
            return Err(format!("未知遥控事件: {event}"));
        }
        target.mappings.insert(event.to_string(), mapping);
        self.save();
        Ok(())
    }

    pub fn set_preset(&mut self, name: &str) -> Result<(), String> {
        if !self.config.presets.contains_key(name) {
            return Err(format!("Preset「{name}」不存在"));
        }
        self.config.preset = name.to_string();
        self.save();
        Ok(())
    }

    pub fn add_preset(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Preset 名称不能为空".into());
        }
        if self.config.presets.contains_key(name) {
            return Err(format!("Preset「{name}」已存在"));
        }
        self.config
            .presets
            .insert(name.to_string(), Preset::with_mappings(codex_mappings()));
        self.save();
        Ok(())
    }

    pub fn delete_preset(&mut self, name: &str) -> Result<(), String> {
        if self.config.presets.len() <= 1 {
            return Err("至少保留一个 Preset".into());
        }
        if self.config.preset == name {
            return Err("不能删除当前启用的 Preset，请先切换".into());
        }
        if self.config.presets.remove(name).is_none() {
            return Err(format!("Preset「{name}」不存在"));
        }
        self.save();
        Ok(())
    }

    pub fn execute(&self, event: &str) -> ActionResult {
        // 耗时打进 detail：diag.json 里能直接看到每键延迟，延迟回归第一时间可见
        let started = std::time::Instant::now();
        let mut result = self.execute_inner(event);
        result
            .detail
            .push_str(&format!("（耗时 {}ms）", started.elapsed().as_millis()));
        result
    }

    fn execute_inner(&self, event: &str) -> ActionResult {
        // 先查映射：没配置的键不值得切焦点
        let Some(mapping) = self.mapping_for(event) else {
            return ActionResult {
                detail: "当前 Preset 未配置该按键".into(),
                ..ActionResult::new(event, "none", "")
            };
        };
        // Preset 可以把按键固定发给某个 App。不这么做的话，按键会落进
        // 「当前前台 App」—— 你盯着 Web Coding 的设置页时，WorkBuddy 什么都收不到，
        // 看起来就像「映射没生效」。
        if let Some(target) = self.target_app() {
            focus_target(&target);
        }
        dispatch(event, &mapping)
    }

    /// 当前 Preset 里配置的目标 App（去空白，空串视为没配）
    fn target_app(&self) -> Option<String> {
        self.config
            .presets
            .get(&self.config.preset)
            .and_then(|p| p.target_app.clone())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// 设置某个 Preset 的目标 App；空串表示清掉（回到「发给当前前台」）
    pub fn set_preset_target(&mut self, preset: &str, target: &str) -> Result<(), String> {
        let target = target.trim();
        let p = self
            .config
            .presets
            .get_mut(preset)
            .ok_or_else(|| format!("Preset「{preset}」不存在"))?;
        p.target_app = if target.is_empty() {
            None
        } else {
            Some(target.to_string())
        };
        self.save();
        Ok(())
    }
}

/// 把焦点切到目标 App，并等到它真的成为前台再返回。
///
/// 快路径全部走进程内 FFI（NSWorkspace / NSRunningApplication），**零子进程、
/// 零 Apple Events**：
/// - 前台判定按 bundle id。WorkBuddy 的 CFBundleExecutable 是 "Electron"，
///   System Events 报的进程名也是 "Electron" —— 按显示名匹配前台永远为 false，
///   会退化成每次按键都 activate + 轮询，这是延迟主根因；
/// - 激活走 NSRunningApplication.activateWithOptions，不依赖 TCC「自动化」授权，
///   重构建后不会被系统弹窗/拒绝阻塞数秒。
/// osascript 只在 FFI 失败或目标 App 没在运行时兜底。
fn focus_target(target: &str) {
    #[cfg(target_os = "macos")]
    {
        if let Some(t) = crate::apps::detector::resolve_running_target(target) {
            if crate::apps::detector::is_front_bundle(&t.bundle_id) {
                return;
            }
            if !crate::apps::detector::activate_pid(t.pid) {
                activate_via_osascript(&t.bundle_id);
            }
            // activate 是异步的：不等待的话，紧跟着的按键会落进切换前的前台 App。
            // 前台查询走进程内 FFI（~0ms），10ms 粒度轮询，切换一完成立刻发键（上限约 400ms）。
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(10));
                if crate::apps::detector::is_front_bundle(&t.bundle_id) {
                    return;
                }
            }
            return;
        }
    }
    // 目标 App 没在运行（或非 macOS）：osascript 激活；失败也不阻塞发键
    activate_via_osascript(target);
}

fn activate_via_osascript(target: &str) {
    // 支持写 App 名（WorkBuddy）或 bundle id（com.tencent.workbuddy.mac）
    let script = if target.contains('.') {
        format!("tell application id \"{target}\" to activate")
    } else {
        format!("tell application \"{target}\" to activate")
    };
    let _ = crate::actions::applescript::run(&script);
}

/// 补齐历史配置。
///
/// 两件必须做的事：
/// 1. 当前 Preset 名不存在时回落到第一个（用户手改坏了配置也不至于起不来）；
/// 2. **把新加的遥控事件补进老 Preset**。配置文件是「首次运行时落盘」的，
///    之后代码里新增事件（例如 `siri`）不会自动出现在老文件里，按键就会显示
///    「当前 Preset 未配置该按键」—— 看起来就像「映射没生效」。
///    只补缺失的槽位，绝不覆盖用户已经改过的值。
fn normalize(mut config: Config) -> Config {
    if config.presets.is_empty() {
        return default_config();
    }
    if !config.presets.contains_key(&config.preset) {
        config.preset = config.presets.keys().next().unwrap().clone();
    }
    let defaults = default_config();
    for (name, preset) in config.presets.iter_mut() {
        let base = defaults
            .presets
            .get(name)
            .map(|p| p.mappings.clone())
            .unwrap_or_else(codex_mappings);
        for (event, mapping) in base {
            preset.mappings.entry(event).or_insert(mapping);
        }
    }
    config
}

fn mapping(kind: MappingKind, value: &str) -> Mapping {
    Mapping {
        kind,
        value: value.to_string(),
    }
}

fn codex_mappings() -> BTreeMap<String, Mapping> {
    BTreeMap::from([
        ("up".into(), mapping(MappingKind::Key, "up")),
        ("down".into(), mapping(MappingKind::Key, "down")),
        ("left".into(), mapping(MappingKind::Key, "left")),
        ("right".into(), mapping(MappingKind::Key, "right")),
        ("center".into(), mapping(MappingKind::Action, "approve")),
        ("back".into(), mapping(MappingKind::Action, "reject")),
        (
            "backLongPress".into(),
            mapping(MappingKind::Action, "voice_input"),
        ),
        (
            "centerLongPress".into(),
            mapping(MappingKind::Action, "voice_input"),
        ),
        ("playPause".into(), mapping(MappingKind::Action, "voice_input")),
        ("siri".into(), mapping(MappingKind::Action, "voice_input")),
    ])
}

/// WorkBuddy 的映射，全部对着它「快捷操作」设置里的真实快捷键来
/// （⌘I 上一个任务 / ⌘J 下一个任务 / ⌘B 切换左侧栏 / ⇧⌘B 切换右侧产物面板 /
///  Enter 发送消息 / Esc 停止生成 / ⌘D 语音录制开关 / ⌘K 全局搜索）。
/// 逻辑是按遥控器的物理直觉分配：左右键管左右两块面板，上下键切任务。
/// 确认/返回走「读 UI 找按钮，找不到回退按键」的审批语义；
/// 长按确认 = 全部允许，长按返回 = 清空输入框（丢掉说错的语音转写）。
fn workbuddy_mappings() -> BTreeMap<String, Mapping> {
    BTreeMap::from([
        ("up".into(), mapping(MappingKind::Shortcut, "Cmd+I")),
        ("down".into(), mapping(MappingKind::Shortcut, "Cmd+J")),
        ("left".into(), mapping(MappingKind::Shortcut, "Cmd+B")),
        (
            "right".into(),
            mapping(MappingKind::Shortcut, "Shift+Cmd+B"),
        ),
        // 对话框在场时点「允许」，不在场时回退 Return 发送消息
        ("center".into(), mapping(MappingKind::Action, "approve")),
        // 对话框在场时点「拒绝」，不在场时回退 Esc 停止生成
        ("back".into(), mapping(MappingKind::Action, "reject")),
        (
            "backLongPress".into(),
            mapping(MappingKind::Action, "clear_input"),
        ),
        (
            "centerLongPress".into(),
            mapping(MappingKind::Action, "approve_all"),
        ),
        // 语音开关宏：Cmd+D 开/停录；停止后转写文本落框时光标停在开头
        // （WorkBuddy 自身行为），补发 Cmd+Down + Cmd+Right 把光标推到末尾。
        // 开始录音时输入框为空，补发无害空操作，两个场景共用一个宏。
        (
            "playPause".into(),
            mapping(
                MappingKind::Macro,
                "Cmd+D|wait:1500|Cmd+Down|Cmd+Right",
            ),
        ),
        ("siri".into(), mapping(MappingKind::Shortcut, "Cmd+K")),
    ])
}

fn default_config() -> Config {
    let mut presets = BTreeMap::new();
    presets.insert("Codex".into(), Preset::with_mappings(codex_mappings()));
    presets.insert("WorkBuddy".into(), Preset::with_mappings(workbuddy_mappings()));
    presets.insert("自定义".into(), Preset::with_mappings(codex_mappings()));
    Config {
        preset: "Codex".into(),
        presets,
        touchpad: TouchpadConfig::default(),
    }
}

/// 把一个映射分派为真实动作（独立函数，便于「测试」命令复用）
pub fn dispatch(event: &str, m: &Mapping) -> ActionResult {
    let base = ActionResult::new(event, m.kind.as_str(), &m.value);
    match &m.kind {
        MappingKind::Key => match keyboard::key_code(&m.value) {
            Some(code) => {
                if !keyboard::accessibility_granted() {
                    return ActionResult {
                        detail: NEED_PERMISSION.into(),
                        ..base
                    };
                }
                keyboard::press_key(code, 0);
                ActionResult {
                    ok: true,
                    detail: format!("已发送按键「{}」", m.value),
                    ..base
                }
            }
            None => ActionResult {
                detail: format!("未知按键名「{}」", m.value),
                ..base
            },
        },
        MappingKind::Shortcut => match keyboard::parse_shortcut(&m.value) {
            Some((code, flags)) => {
                if !keyboard::accessibility_granted() {
                    return ActionResult {
                        detail: NEED_PERMISSION.into(),
                        ..base
                    };
                }
                keyboard::press_key(code, flags);
                ActionResult {
                    ok: true,
                    detail: format!("已发送快捷键「{}」", m.value),
                    ..base
                }
            }
            None => ActionResult {
                detail: format!("无法解析快捷键「{}」，示例：Cmd+Enter", m.value),
                ..base
            },
        },
        MappingKind::Action => match m.value.as_str() {
            "approve" => {
                match accessibility::click_frontmost_button(
                    accessibility::APPROVE_WORDS,
                    APPROVE_TIMEOUT,
                ) {
                    Ok(name) => ActionResult {
                        ok: true,
                        detail: format!("已点击按钮 {name}"),
                        ..base
                    },
                    Err(_) => {
                        keyboard::press_key(keyboard::key_code("return").unwrap(), 0);
                        ActionResult {
                            ok: true,
                            detail: "未找到批准按钮，已回退发送 Return".into(),
                            ..base
                        }
                    }
                }
            }
            "reject" => {
                match accessibility::click_frontmost_button(
                    accessibility::REJECT_WORDS,
                    REJECT_TIMEOUT,
                ) {
                    Ok(name) => ActionResult {
                        ok: true,
                        detail: format!("已点击按钮 {name}"),
                        ..base
                    },
                    Err(_) => {
                        keyboard::press_key(keyboard::key_code("escape").unwrap(), 0);
                        ActionResult {
                            ok: true,
                            detail: "未找到拒绝按钮，已回退发送 Escape".into(),
                            ..base
                        }
                    }
                }
            }
            // 「全部允许」没有按键回退：对话框不在场时什么都不做，
            // 盲发按键可能误触别的确认语义。
            "approve_all" => {
                match accessibility::click_frontmost_button(
                    accessibility::APPROVE_ALL_WORDS,
                    APPROVE_ALL_TIMEOUT,
                ) {
                    Ok(name) => ActionResult {
                        ok: true,
                        detail: format!("已点击按钮 {name}"),
                        ..base
                    },
                    Err(e) => ActionResult {
                        detail: format!("未点击「全部允许」：{e}"),
                        ..base
                    },
                }
            }
            // 清空输入框 = 丢掉说错的语音转写。发 Cmd+A + Delete，
            // 要求焦点在输入框上（语音转写完成后通常满足）。
            "clear_input" => {
                if !keyboard::accessibility_granted() {
                    return ActionResult {
                        detail: NEED_PERMISSION.into(),
                        ..base
                    };
                }
                keyboard::press_key(keyboard::key_code("a").unwrap(), keyboard::FLAG_COMMAND);
                std::thread::sleep(std::time::Duration::from_millis(30));
                keyboard::press_key(keyboard::key_code("delete").unwrap(), 0);
                ActionResult {
                    ok: true,
                    detail: "已清空输入框（Cmd+A + Delete）".into(),
                    ..base
                }
            }
            "continue" => {
                keyboard::press_key(keyboard::key_code("return").unwrap(), 0);
                ActionResult {
                    ok: true,
                    detail: "已发送 Return（continue）".into(),
                    ..base
                }
            }
            "send" => match keyboard::parse_shortcut("Cmd+Enter") {
                Some((code, flags)) => {
                    keyboard::press_key(code, flags);
                    ActionResult {
                        ok: true,
                        detail: "已发送 Cmd+Enter（send）".into(),
                        ..base
                    }
                }
                None => unreachable!(),
            },
            "voice_input" => {
                keyboard::double_tap_control();
                ActionResult {
                    ok: true,
                    detail: "已模拟双击 Control 触发听写".into(),
                    ..base
                }
            }
            other => ActionResult {
                detail: format!("未知内置动作「{other}」，可选：{:?}", BUILTIN_ACTIONS),
                ..base
            },
        },
        MappingKind::Applescript => match applescript::run(&m.value) {
            Ok(out) => ActionResult {
                ok: true,
                detail: if out.is_empty() { "AppleScript 执行成功".into() } else { out },
                ..base
            },
            Err(e) => ActionResult {
                detail: format!("AppleScript 失败：{e}"),
                ..base
            },
        },
        MappingKind::Shell => match shell::run(&m.value) {
            Ok(out) => ActionResult {
                ok: true,
                detail: if out.is_empty() { "命令执行成功".into() } else { out },
                ..base
            },
            Err(e) => ActionResult {
                detail: format!("命令失败：{e}"),
                ..base
            },
        },
        MappingKind::Macro => match parse_macro(&m.value) {
            Some(steps) => {
                if !keyboard::accessibility_granted() {
                    return ActionResult {
                        detail: NEED_PERMISSION.into(),
                        ..base
                    };
                }
                for step in &steps {
                    match step {
                        MacroStep::Wait(ms) => {
                            std::thread::sleep(std::time::Duration::from_millis(*ms))
                        }
                        MacroStep::Press(code, flags) => keyboard::press_key(*code, *flags),
                    }
                }
                ActionResult {
                    ok: true,
                    detail: format!("已执行宏（{} 步）", steps.len()),
                    ..base
                }
            }
            None => ActionResult {
                detail: format!(
                    "宏解析失败：「{}」。步骤用 | 分隔，每步是 wait:毫秒 或快捷键/按键名",
                    m.value
                ),
                ..base
            },
        },
    }
}

enum MacroStep {
    Wait(u64),
    Press(u16, u64),
}

/// 解析宏序列："Cmd+D|wait:1500|Cmd+Down|Cmd+Right"。
/// 先整体校验再执行：任何一步解析不了都返回 None，不执行半截宏。
fn parse_macro(value: &str) -> Option<Vec<MacroStep>> {
    let mut steps = Vec::new();
    for part in value.split('|') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(rest) = part.strip_prefix("wait:") {
            let ms: u64 = rest.trim().parse().ok()?;
            steps.push(MacroStep::Wait(ms));
        } else if let Some((code, flags)) = keyboard::parse_shortcut(part) {
            steps.push(MacroStep::Press(code, flags));
        } else {
            steps.push(MacroStep::Press(keyboard::key_code(part)?, 0));
        }
    }
    if steps.is_empty() {
        None
    } else {
        Some(steps)
    }
}

const NEED_PERMISSION: &str =
    "需要辅助功能权限：系统设置 → 隐私与安全性 → 辅助功能 → 允许 WebCoding";

/// AX 找按钮的超时护栏。`entire contents` 在大窗口（Electron）上可能秒级，
/// 不设上限会把 approve 类动作的延迟拖到用户可感知。
/// approve/reject 是热路径（确认/返回键），护栏更紧；approve_all 低频，放宽换准确。
const APPROVE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(600);
const REJECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(600);
const APPROVE_ALL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::keyboard::{key_code, parse_shortcut};

    #[test]
    fn default_config_has_all_events() {
        let config = default_config();
        for (name, preset) in &config.presets {
            assert_eq!(preset.mappings.len(), REMOTE_EVENTS.len(), "preset {name}");
            for ev in REMOTE_EVENTS {
                assert!(preset.mappings.contains_key(ev), "preset {name} 缺少 {ev}");
            }
        }
    }

    #[test]
    fn dispatch_key_and_shortcut() {
        let r = dispatch("up", &mapping(MappingKind::Key, "up"));
        assert!(r.detail.contains("up") || r.ok || r.detail.contains("辅助功能"));
        let bad = dispatch("center", &mapping(MappingKind::Shortcut, "not-a-key+"));
        assert!(!bad.ok);
        assert!(bad.detail.contains("无法解析"));
    }

    /// 老配置文件缺新事件槽位时，必须自动补上；用户改过的值不能被覆盖。
    #[test]
    fn legacy_config_gets_missing_events_backfilled() {
        let legac = Config {
            preset: "WorkBuddy".into(),
            presets: BTreeMap::from([(
                "WorkBuddy".into(),
                Preset::with_mappings(BTreeMap::from([
                    // 用户自己改过的值
                    ("center".into(), mapping(MappingKind::Key, "return")),
                    ("up".into(), mapping(MappingKind::Key, "up")),
                ])),
            )]),
            touchpad: TouchpadConfig::default(),
        };
        let fixed = normalize(legac);
        let preset = fixed.presets.get("WorkBuddy").unwrap();
        // 缺的槽位补齐
        for ev in REMOTE_EVENTS {
            assert!(preset.mappings.contains_key(ev), "补完后仍缺 {ev}");
        }
        // 用户改过的值原样保留，不被默认值覆盖
        assert_eq!(preset.mappings.get("center").unwrap().value, "return");
        assert_eq!(
            preset.mappings.get("center").unwrap().kind,
            MappingKind::Key
        );
    }

    /// Preset 可以固定目标 App；空串表示清掉。老配置没有 targetApp 字段时必须是 None。
    #[test]
    fn preset_target_app_can_be_set_and_cleared() {
        let mut config = default_config();
        config.preset = "WorkBuddy".into();
        let path = std::env::temp_dir().join(format!(
            "webcoding-test-target-{}.json",
            std::process::id()
        ));
        let mut engine = Engine {
            config,
            path: path.clone(),
        };

        // 老配置（没有 targetApp 字段）反序列化后就是 None，不会误发焦点切换
        assert!(engine.target_app().is_none());

        engine
            .set_preset_target("WorkBuddy", "com.tencent.workbuddy.mac")
            .unwrap();
        assert_eq!(
            engine.target_app().as_deref(),
            Some("com.tencent.workbuddy.mac")
        );
        // 落盘后再读回来仍然在（走 serde camelCase: targetApp）
        let reloaded = Engine::load(path.clone());
        assert_eq!(
            reloaded.target_app().as_deref(),
            Some("com.tencent.workbuddy.mac")
        );

        engine.set_preset_target("WorkBuddy", "  ").unwrap();
        assert!(engine.target_app().is_none(), "空串应清掉目标");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn workbuddy_preset_matches_the_apps_real_shortcuts() {
        // 对照 WorkBuddy「快捷操作」设置里的真实绑定，别想当然
        let config = default_config();
        let m = |event: &str| config.presets["WorkBuddy"].mappings.get(event).unwrap();
        let shortcut = |event: &str| match m(event) {
            Mapping { kind: MappingKind::Shortcut, value } => value.as_str(),
            other => panic!("{event} 应该是 shortcut，实际 {other:?}"),
        };
        let action = |event: &str, want: &str| match m(event) {
            Mapping { kind: MappingKind::Action, value } => assert_eq!(value, want),
            other => panic!("{event} 应该是 action {want}，实际 {other:?}"),
        };
        assert_eq!(shortcut("up"), "Cmd+I", "上一个任务");
        assert_eq!(shortcut("down"), "Cmd+J", "下一个任务");
        assert_eq!(shortcut("left"), "Cmd+B", "切换左侧栏");
        assert_eq!(shortcut("right"), "Shift+Cmd+B", "切换右侧产物面板");
        assert_eq!(shortcut("siri"), "Cmd+K", "全局搜索");
        // 语音开关是宏：Cmd+D + 延时 + 光标回末尾
        match m("playPause") {
            Mapping { kind: MappingKind::Macro, value } => {
                assert!(value.starts_with("Cmd+D"), "语音开关宏必须以 Cmd+D 开头");
                assert!(value.contains("wait:"), "语音开关宏必须带延时");
                assert!(value.contains("Cmd+Down") && value.contains("Cmd+Right"), "宏必须把光标推到末尾");
            }
            other => panic!("playPause 应该是 macro，实际 {other:?}"),
        }
        // 审批语义：读 UI 找按钮，找不到回退对应按键
        action("center", "approve");
        action("back", "reject");
        action("centerLongPress", "approve_all");
        action("backLongPress", "clear_input");
        // 每个快捷键都必须真的能解析成键码，否则按键会静默失败
        for event in REMOTE_EVENTS {
            let mapping = m(event);
            let parsed = match &mapping.kind {
                MappingKind::Shortcut => parse_shortcut(&mapping.value).is_some(),
                MappingKind::Key => key_code(&mapping.value).is_some(),
                MappingKind::Macro => parse_macro(&mapping.value).is_some(),
                _ => true,
            };
            assert!(parsed, "{event} 的快捷键「{}」解析不了", mapping.value);
        }
    }

    /// 宏序列解析：wait 步、快捷键步、裸按键名步；任何一步坏整体返回 None
    #[test]
    fn parses_macro_steps() {
        let steps = parse_macro("Cmd+D|wait:1500|Cmd+Down|Cmd+Right").unwrap();
        assert_eq!(steps.len(), 4);
        // 裸按键名
        assert_eq!(parse_macro("return|wait:100").unwrap().len(), 2);
        // 坏步骤 → 整体 None，不执行半截
        assert!(parse_macro("Cmd+D|wait:abc").is_none());
        assert!(parse_macro("nope+|return").is_none());
        assert!(parse_macro("   ").is_none());
    }

    /// 两个新内置动作必须在合法清单里，UI 下拉框和引擎校验共用这份清单
    #[test]
    fn builtin_actions_cover_the_dialog_and_voice_flows() {
        assert!(BUILTIN_ACTIONS.contains(&"approve_all"));
        assert!(BUILTIN_ACTIONS.contains(&"clear_input"));
    }

    #[test]
    fn config_roundtrip() {
        let config = default_config();
        let text = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.preset, "Codex");
        assert_eq!(back.presets.len(), 3);
    }
}
