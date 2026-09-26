// 全局自动更新：App 启动即静默检查（不再依赖关于页是否被打开），
// 发现新版本自动下载安装重启；关于页与全局横幅共用这一份状态。
import { create } from "zustand";
import { check as checkUpdater, type Update } from "@tauri-apps/plugin-updater";
import { invoke } from "@tauri-apps/api/core";

const REPO = "https://github.com/hellojerry001/vibe-remote";
const AUTO_CHECK_KEY = "viberemote.autoCheck";
/** 与 tauri.conf.json 的 updater.endpoints 保持一致；内置更新器失败时的兜底入口 */
const UPDATE_JSON = "https://raw.githubusercontent.com/hellojerry001/vibe-remote/main/update.json";

/** 比较两个 x.y.z 版本号，>0 表示 a 更新 */
export function cmpVersion(a: string, b: string): number {
  const pa = a.replace(/^v/i, "").split(".").map((n) => parseInt(n, 10) || 0);
  const pb = b.replace(/^v/i, "").split(".").map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < 3; i += 1) {
    if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) - (pb[i] ?? 0);
  }
  return 0;
}

/** 兜底：直接读 update.json。null = 网络失败；hasUpdate 由调用方与当前版本比较 */
async function fetchLatestVersion(appVersion: string): Promise<{ version: string; hasUpdate: boolean } | "none" | null> {
  try {
    const res = await fetch(UPDATE_JSON, { cache: "no-store" });
    if (res.status === 404) return "none";
    if (!res.ok) return null;
    const j = (await res.json()) as { version?: unknown };
    const v = typeof j.version === "string" ? j.version.replace(/^v/i, "") : null;
    if (!v) return null;
    return { version: v, hasUpdate: cmpVersion(v, appVersion) > 0 };
  } catch {
    return null;
  }
}

export type UpdaterPhase = "idle" | "checking" | "downloading";

interface UpdaterStore {
  phase: UpdaterPhase;
  latest: string | null;
  hasNew: boolean;
  /** 0..1，仅 downloading 阶段有意义 */
  progress: number;
  /** 更新失败详情（横幅/轻提示展示用） */
  errMsg: string | null;
  autoCheck: boolean;
  toast: string | null;
  /** 当前 App 版本（About 页拿到 get_app_version 后回填，兜底比较用） */
  appVersion: string;
  setAppVersion: (v: string) => void;
  setAutoCheck: (on: boolean) => void;
  /** silent=true 时发现新版本直接下载安装，且不弹轻提示 */
  check: (silent: boolean) => Promise<void>;
  install: () => Promise<void>;
  showToast: (msg: string) => void;
  clearToast: () => void;
}

let pendingUpdate: Update | null = null;
let autoRan = false;
let s_autoPending = false;
let toastTimer: number | null = null;

export const useUpdater = create<UpdaterStore>((set, get) => ({
  phase: "idle",
  latest: null,
  hasNew: false,
  progress: 0,
  errMsg: null,
  autoCheck: localStorage.getItem(AUTO_CHECK_KEY) !== "0",
  toast: null,
  appVersion: "",

  setAppVersion: (v) => {
    const first = get().appVersion === "" && v !== "";
    set({ appVersion: v });
    // 开关开着但启动检查因为版本号未就绪而跳过时，这里补跑一次
    if (first && s_autoPending) {
      s_autoPending = false;
      void get().check(true).catch(() => {});
    }
  },

  setAutoCheck: (on) => {
    set({ autoCheck: on });
    localStorage.setItem(AUTO_CHECK_KEY, on ? "1" : "0");
    if (on && !autoRan && get().appVersion) {
      autoRan = true;
      void get().check(true).catch(() => {});
    }
  },

  check: async (silent) => {
    if (get().phase !== "idle") return;
    set({ phase: "checking", errMsg: null });
    try {
      pendingUpdate = await checkUpdater();
      set({ hasNew: !!pendingUpdate, latest: pendingUpdate?.version ?? null });
      if (pendingUpdate && silent) {
        void get().install();
      }
    } catch {
      // 内置更新器失败（常见：无网络/代理不通）→ 退回读 update.json，
      // 这条路只能「知道有新版本」，真正下载仍走 install() 里重试 checkUpdater
      const tag = await fetchLatestVersion(get().appVersion);
      if (tag === "none") {
        set({ hasNew: false, latest: null });
        if (!silent) get().showToast("当前已是最新版本");
      } else if (tag) {
        set({ hasNew: tag.hasUpdate, latest: tag.version });
        if (!silent && !tag.hasUpdate) get().showToast("当前已是最新版本");
      } else {
        set({ hasNew: false, errMsg: "检查失败，请稍后再试" });
        if (!silent) get().showToast("检查失败，请稍后再试");
      }
    } finally {
      set({ phase: "idle" });
    }
  },

  install: async () => {
    if (get().phase === "downloading") return;
    set({ phase: "downloading", progress: 0, errMsg: null });
    try {
      if (!pendingUpdate) {
        pendingUpdate = await checkUpdater();
        if (pendingUpdate) set({ latest: pendingUpdate.version, hasNew: true });
      }
      if (!pendingUpdate) {
        set({ hasNew: false, phase: "idle" });
        return;
      }
      let total = 0;
      let downloaded = 0;
      await pendingUpdate.downloadAndInstall((e) => {
        if (e.event === "Started") {
          total = e.data.contentLength ?? 0;
        } else if (e.event === "Progress") {
          downloaded += e.data.chunkLength;
          if (total > 0) set({ progress: Math.min(1, downloaded / total) });
        }
      });
      // macOS 安装完成后必须重启才会跑新版本
      await invoke("restart_app");
    } catch (e) {
      // 最常见的失败是网络中断 / 签名校验不过：给明确原因，回退打开 Releases 页
      const raw = e instanceof Error ? e.message : String(e);
      const msg = raw.toLowerCase();
      const unsigned = msg.includes("signature") || msg.includes("verify") || msg.includes("not signed");
      const text = (unsigned ? "更新包未通过签名校验：" : "更新失败：") + raw.replace(/\s+/g, " ").slice(0, 90);
      set({ phase: "idle", errMsg: text });
      get().showToast(text);
      pendingUpdate = null;
      void invoke("open_url", { url: `${REPO}/releases` }).catch(() => {});
    }
  },

  showToast: (msg) => {
    set({ toast: msg });
    if (toastTimer) window.clearTimeout(toastTimer);
    // 失败信息比较长，多留几秒看清原因
    toastTimer = window.setTimeout(() => set({ toast: null }), 8000);
  },

  clearToast: () => {
    if (toastTimer) window.clearTimeout(toastTimer);
    set({ toast: null });
  },
}));

/** App 启动时调一次：开关打开且版本号已就绪就静默检查，否则等 setAppVersion 回填后补跑 */
export function startAutoUpdate(): void {
  const s = useUpdater.getState();
  if (!s.autoCheck || autoRan) return;
  if (!s.appVersion) {
    s_autoPending = true;
    return;
  }
  autoRan = true;
  void s.check(true).catch(() => {});
}
