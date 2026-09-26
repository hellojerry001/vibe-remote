import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  ActionResult,
  BtDevice,
  Config,
  ConnectionState,
  EventRow,
  HidEvent,
  MappingKind,
  RemoteEvent,
  Status,
} from "../types";

interface AppStore {
  config: Config | null;
  status: Status | null;
  events: EventRow[];
  lastHid: HidEvent | null;
  /** 十值连接状态机快照（三层读数） */
  connection: ConnectionState | null;
  found: BtDevice[];
  /** 扫描过程中的补充提示（蓝牙关闭、命令失败等） */
  scanNote: string | null;
  loadConfig: () => Promise<void>;
  refreshStatus: () => Promise<void>;
  setConnection: (c: ConnectionState) => void;
  addFoundDevice: (d: BtDevice) => void;
  clearFoundDevices: () => void;
  setScanNote: (note: string | null) => void;
  refreshConnection: () => Promise<void>;
  startScan: () => Promise<void>;
  stopScan: () => Promise<void>;
  setStatus: (status: Status) => void;
  setLastHid: (hid: HidEvent) => void;
  pushEvent: (event: ActionResult) => void;
  setMapping: (preset: string, event: RemoteEvent, kind: MappingKind, value: string) => Promise<void>;
  setPreset: (name: string) => Promise<void>;
  setPresetTarget: (preset: string, target: string) => Promise<void>;
  addPreset: (name: string) => Promise<void>;
  deletePreset: (name: string) => Promise<void>;
  testAction: (kind: MappingKind, value: string) => Promise<ActionResult>;
}

export const useAppStore = create<AppStore>((set, get) => ({
  config: null,
  status: null,
  events: [],
  lastHid: null,
  connection: null,
  found: [],
  scanNote: null,

  loadConfig: async () => {
    set({ config: await invoke<Config>("get_config") });
  },

  refreshStatus: async () => {
    set({ status: await invoke<Status>("get_status") });
  },

  setStatus: (status) => set({ status }),

  setConnection: (connection) => set({ connection }),

  addFoundDevice: (d) =>
    set((s) =>
      s.found.some((x) => x.address === d.address) ? s : { found: [...s.found, d] },
    ),

  clearFoundDevices: () => set({ found: [] }),

  setScanNote: (scanNote) => set({ scanNote }),

  refreshConnection: async () => {
    try {
      set({ connection: await invoke<ConnectionState>("get_connection_state") });
    } catch {
      /* 状态读取失败不影响主流程 */
    }
  },

  startScan: async () => {
    set({ found: [], scanNote: null });
    await invoke("bt_start_scan");
  },

  stopScan: async () => {
    await invoke("bt_stop_scan");
  },

  setLastHid: (hid) => set({ lastHid: hid }),

  pushEvent: (event) =>
    set((s) => ({
      events: [{ ...event, ts: new Date().toLocaleTimeString() }, ...s.events].slice(0, 50),
    })),

  setMapping: async (preset, event, kind, value) => {
    await invoke("set_mapping", { preset, event, kind, value });
    await get().loadConfig();
  },

  setPreset: async (name) => {
    await invoke("set_preset", { name });
    await get().loadConfig();
  },

  setPresetTarget: async (preset, target) => {
    await invoke("set_preset_target", { input: { preset, target } });
    await get().loadConfig();
  },

  addPreset: async (name) => {
    await invoke("add_preset", { name });
    await get().loadConfig();
  },

  deletePreset: async (name) => {
    await invoke("delete_preset", { name });
    await get().loadConfig();
  },

  testAction: (kind, value) =>
    invoke<ActionResult>("test_action", { action: { kind, value } }),
}));
