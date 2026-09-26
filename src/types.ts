export type MappingKind = "shortcut" | "key" | "action" | "applescript" | "shell" | "macro";

export interface Mapping {
  kind: MappingKind;
  value: string;
}

export interface Preset {
  mappings: Record<string, Mapping>;
  /** 固定把按键发给哪个 App（App 名或 bundle id）。空/缺省 = 发给当前前台 App */
  targetApp?: string | null;
}

export interface TouchpadConfig {
  enabled: boolean;
  gain: number;
  tapClick: boolean;
}

export interface Config {
  touchpad: TouchpadConfig;
  preset: string;
  presets: Record<string, Preset>;
}

export interface HidEvent {
  event: string;
  usagePage: number;
  usage: number;
  value: number;
  pressed: boolean;
}

/** 权限状态由 Native 层真实读取，前端只做展示，不做推断 */
export type PermissionLevel = "granted" | "denied" | "unknown";

export interface SystemStatus {
  inputMonitoring: PermissionLevel;
  accessibility: PermissionLevel;
  hidManagerOpen: boolean;
  inputCallbackActive: boolean;
  a2854Matched: boolean;
  matchedDeviceCount: number;
  vendorId: string;
  /** 实际匹配到的遥控器 PID；未匹配到为 null（不回填期望值） */
  productId: string | null;
  /** 匹配白名单：没匹配到时用它说明「在等什么」 */
  productIdWhitelist: string[];
  appPath: string;
  bundleId: string;
  lastHidEvent: HidEvent | null;
}

/** 十值连接状态机，全部由 Rust 侧真实读数驱动 */
export type RemoteMachine =
  | "idle"
  | "instructions"
  | "scanning"
  | "found"
  | "pairing"
  | "paired"
  | "connecting"
  | "connected"
  | "hid_ready"
  | "failed";

/** 单层的通断程度，与机器状态互相独立 */
export type LayerLevel = "off" | "pending" | "working" | "error";

export interface ConnectionState {
  machine: RemoteMachine;
  machineLabel: string;
  step: number | null;
  bluetooth: LayerLevel;
  bluetoothDetail: string;
  connection: LayerLevel;
  connectionDetail: string;
  hid: LayerLevel;
  hidDetail: string;
  a2854Matched: boolean;
  error: string | null;
}

/** 扫描到的蓝牙设备（数据来自 system_profiler，不含 RSSI） */
export interface BtDevice {
  address: string;
  name: string;
  /** 在「已连接」分组里；false = 已配对过但链路断开，或刚进入配对模式 */
  connected: boolean;
  isSiriRemote: boolean;
  vendorId: number | null;
  productId: number | null;
}

export interface Status {
  touchpad: { source: string; devices: number; error: number; frames: number; pointerMoves: number };
  running: boolean;
  port: number;
  tvConnected: boolean;
  accessibility: boolean;
  frontApp: string;
  /** A2854 蓝牙直连检测到的设备数 */
  connectedDevices: number;
  hidError: string | null;
  lastEvent: string | null;
}

export interface ActionResult {
  event: string;
  kind: string;
  value: string;
  ok: boolean;
  detail: string;
}

export interface EventRow extends ActionResult {
  ts: string;
}

export const REMOTE_EVENTS = [
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
] as const;

export type RemoteEvent = (typeof REMOTE_EVENTS)[number];

export const EVENT_LABELS: Record<RemoteEvent, string> = {
  up: "上",
  down: "下",
  left: "左",
  right: "右",
  center: "确认键",
  back: "返回键",
  backLongPress: "返回长按",
  playPause: "播放 / 暂停",
  centerLongPress: "确认长按",
  siri: "Siri 键",
};

export const KIND_LABELS: Record<MappingKind, string> = {
  shortcut: "快捷键",
  key: "单键",
  action: "内置动作",
  applescript: "AppleScript",
  shell: "Shell 命令",
  macro: "宏序列",
};

export const BUILTIN_ACTIONS = [
  "approve",
  "reject",
  "approve_all",
  "clear_input",
  "continue",
  "send",
  "voice_input",
] as const;

export const ACTION_LABELS: Record<string, string> = {
  approve: "批准（找按钮 / 回退 Return）",
  reject: "拒绝（找按钮 / 回退 Escape）",
  approve_all: "全部允许（找按钮，无回退）",
  clear_input: "清空输入框（Cmd+A + Delete）",
  continue: "继续（Return）",
  send: "发送（Cmd+Enter）",
  voice_input: "语音输入（双击 Control）",
};
