// About and startup share one update operation, including download/install/restart.
import { create } from "zustand";
import { check as checkUpdater, type Update } from "@tauri-apps/plugin-updater";
import { invoke } from "@tauri-apps/api/core";

const AUTO_CHECK_KEY = "viberemote.autoCheck";
export type UpdaterPhase = "idle" | "checking" | "downloading" | "installing" | "restarting";

interface UpdaterStore {
  phase: UpdaterPhase;
  latest: string | null;
  hasNew: boolean;
  checked: boolean;
  installed: boolean;
  progress: number;
  errMsg: string | null;
  autoCheck: boolean;
  toast: string | null;
  appVersion: string;
  setAppVersion: (v: string) => void;
  setAutoCheck: (on: boolean) => void;
  check: (silent: boolean) => Promise<void>;
  install: () => Promise<void>;
  showToast: (msg: string) => void;
  clearToast: () => void;
}

let pendingUpdate: Update | null = null;
let autoRan = false;
let autoPending = false;
let toastTimer: number | null = null;

async function closePending() {
  const update = pendingUpdate;
  pendingUpdate = null;
  await update?.close().catch(() => {});
}
function errorText(prefix: string, error: unknown) {
  const raw = error instanceof Error ? error.message : String(error);
  return prefix + raw.replace(/\s+/g, " ").slice(0, 160);
}

export const useUpdater = create<UpdaterStore>((set, get) => ({
  phase: "idle",
  latest: null,
  hasNew: false,
  checked: false,
  installed: false,
  progress: 0,
  errMsg: null,
  autoCheck: localStorage.getItem(AUTO_CHECK_KEY) !== "0",
  toast: null,
  appVersion: "",

  setAppVersion: (v) => {
    set({ appVersion: v });
    if (v && autoPending) startAutoUpdate();
  },
  setAutoCheck: (on) => {
    set({ autoCheck: on });
    localStorage.setItem(AUTO_CHECK_KEY, on ? "1" : "0");
    if (on) startAutoUpdate();
    else autoPending = false;
  },

  check: async (silent) => {
    if (get().phase !== "idle" || get().installed) return;
    set({ phase: "checking", errMsg: null, toast: null });
    try {
      await closePending();
      pendingUpdate = await checkUpdater({ timeout: 20000 });
      set({ checked: true, hasNew: !!pendingUpdate, latest: pendingUpdate?.version ?? null, phase: "idle" });
    } catch (error) {
      // A network/404/metadata failure is not evidence that the app is up to date.
      const text = errorText("检查更新失败：", error);
      set({ phase: "idle", checked: false, hasNew: false, latest: null, errMsg: text });
      if (!silent) get().showToast(text);
      return;
    }
    if (!pendingUpdate) {
      if (!silent) get().showToast("当前已是最新版本");
    } else if (silent && get().autoCheck) {
      await get().install();
    } else if (!silent) {
      get().showToast(`发现新版本 v${pendingUpdate.version}，点击「立即更新」下载安装`);
    }
  },

  install: async () => {
    if (get().phase !== "idle") return;
    set({ phase: get().installed ? "restarting" : "downloading", progress: 0, errMsg: null, toast: null });
    try {
      if (!get().installed) {
        if (!pendingUpdate) pendingUpdate = await checkUpdater({ timeout: 20000 });
        if (!pendingUpdate) {
          set({ hasNew: false, latest: null, checked: true, phase: "idle" });
          get().showToast("当前已是最新版本");
          return;
        }
        set({ latest: pendingUpdate.version, hasNew: true });
        let total = 0;
        let downloaded = 0;
        await pendingUpdate.downloadAndInstall((event) => {
          if (event.event === "Started") {
            total = event.data.contentLength ?? 0;
            downloaded = 0;
          } else if (event.event === "Progress") {
            downloaded += event.data.chunkLength;
            if (total > 0) set({ progress: Math.min(1, downloaded / total) });
          } else if (event.event === "Finished") {
            set({ phase: "installing", progress: 1 });
          }
        }, { timeout: 300000 });
        set({ installed: true, progress: 1 });
        await closePending();
      }
      set({ phase: "restarting" });
      await invoke("restart_app");
    } catch (error) {
      const text = errorText(get().installed ? "更新已安装，重启失败，请重试或手动重启：" : "更新失败：", error);
      await closePending();
      set({ phase: "idle", errMsg: text });
      get().showToast(text);
    }
  },

  showToast: (msg) => {
    if (toastTimer !== null) window.clearTimeout(toastTimer);
    set({ toast: msg });
    toastTimer = window.setTimeout(() => set({ toast: null }), 8000);
  },
  clearToast: () => {
    if (toastTimer !== null) window.clearTimeout(toastTimer);
    set({ toast: null });
  },
}));

export function startAutoUpdate(): void {
  const state = useUpdater.getState();
  if (!state.autoCheck || autoRan) return;
  if (!state.appVersion) { autoPending = true; return; }
  autoPending = false;
  autoRan = true;
  void state.check(true);
}
