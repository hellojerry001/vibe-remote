import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import Mappings from "./pages/Mappings";
import About from "./pages/About";
import Presets from "./pages/Presets";
import Remote from "./pages/Remote";
import { useAppStore } from "./stores/useAppStore";
import { startAutoUpdate, useUpdater } from "./updater";
import type { ActionResult, BtDevice, ConnectionState, HidEvent, Status } from "./types";

const TABS = [
  { id: "remote", label: "设备状态", icon: IconRemote },
  { id: "mappings", label: "遥控器映射", icon: IconSliders },
  { id: "presets", label: "Agent Preset", icon: IconLayers },
] as const;

type TabId = (typeof TABS)[number]["id"] | "about";

export default function App() {
  // 支持 #about 深链直达（初始一次即可）
  const [tab, setTab] = useState<TabId>(() => {
    const h = window.location.hash.slice(1) as TabId;
    return h === "remote" || h === "mappings" || h === "presets" || h === "about" ? h : "remote";
  });
  const loadConfig = useAppStore((s) => s.loadConfig);
  const refreshStatus = useAppStore((s) => s.refreshStatus);
  const setStatus = useAppStore((s) => s.setStatus);
  const setLastHid = useAppStore((s) => s.setLastHid);
  const pushEvent = useAppStore((s) => s.pushEvent);
  const setConnection = useAppStore((s) => s.setConnection);
  const addFoundDevice = useAppStore((s) => s.addFoundDevice);
  const setScanNote = useAppStore((s) => s.setScanNote);
  const refreshConnection = useAppStore((s) => s.refreshConnection);

  // 全局自动更新：启动即检查（此前只在关于页挂载时才查，等于从不自动检查）
  const updater = useUpdater();
  useEffect(() => {
    invoke<string>("get_app_version")
      .then((v) => {
        useUpdater.getState().setAppVersion(v);
        startAutoUpdate();
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    loadConfig();
    refreshStatus();
    const unlisteners = [
      listen<Status>("server-status", (e) => setStatus(e.payload)),
      listen<ActionResult>("remote-event", (e) => pushEvent(e.payload)),
      // A2854 蓝牙直连：原始 HID 事件仅用于诊断，不进指令流
      listen<HidEvent>("siri-remote-input", (e) => setLastHid(e.payload)),
      listen<number>("siri-remote-connection", () => refreshStatus()),
      listen<string>("siri-remote-error", () => refreshStatus()),
      // 补授权后看门狗自动恢复，清掉错误提示
      listen<string>("siri-remote-ready", () => refreshStatus()),
      // 连接状态机快照：十值机 + 蓝牙 / 连接 / HID 三层读数
      listen<ConnectionState>("remote-connection-state", (e) => setConnection(e.payload)),
      listen<BtDevice>("bt-device-found", (e) => addFoundDevice(e.payload)),
      listen<string>("bt-scan-note", (e) => setScanNote(e.payload)),
      listen("bt-scan-finished", () => refreshConnection()),
    ];
    return () => {
      unlisteners.forEach((p) => p.then((fn) => fn()));
    };
  }, [
    loadConfig,
    refreshStatus,
    setStatus,
    pushEvent,
    setLastHid,
    setConnection,
    addFoundDevice,
    setScanNote,
    refreshConnection,
  ]);

  // 拖动窗口：侧边栏空白 + 内容区顶部一带（前 ~100px）可拖；
  // 命中按钮/输入类控件不拖。不渲染可见标题栏，避免顶部出现灰条。
  useEffect(() => {
    const onMouseDown = (e: MouseEvent) => {
      const el = e.target as HTMLElement | null;
      if (!el) return;
      if (el.closest("button, input, select, textarea, a, label, [data-no-drag]")) return;
      const inSidebar = !!el.closest(".sidebar");
      const inTopBand = e.clientY <= 100 && !!el.closest(".content");
      if (!inSidebar && !inTopBand) return;
      void invoke("start_dragging").catch(() => {});
    };
    window.addEventListener("mousedown", onMouseDown);
    return () => window.removeEventListener("mousedown", onMouseDown);
  }, []);

  return (
    <div className="layout">
      <aside className="sidebar">
          <div className="brand">
          <span className="brand-icon">
            {/* 与启动图标同源：src-tauri/icons/icon.png → public/app-icon.png */}
            <img src="/app-icon.png" alt="" draggable={false} />
          </span>
          VibeRemote
        </div>
        <nav className="nav">
            {TABS.map((t) => (
              <button
                key={t.id}
                className={tab === t.id ? "nav-item active" : "nav-item"}
                onClick={() => setTab(t.id)}
              >
              <t.icon />
              {t.label}
            </button>
          ))}
        </nav>
        <SidebarAbout active={tab === "about"} onClick={() => setTab("about")} />
      </aside>
      <main className="content">
          {tab === "remote" && <Remote />}
          {tab === "mappings" && <Mappings />}
          {tab === "presets" && <Presets />}
          {tab === "about" && <About />}
      </main>
      {updater.toast && (
        <div className="about-toast" role="status" aria-live="polite" data-no-drag>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true" style={{ flexShrink: 0 }}>
            <circle cx="8" cy="8" r="8" fill="#34A853" />
            <path d="M4.8 8.1 7 10.3 11.2 5.8" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          {updater.toast}
        </div>
      )}
      {/* 下载更新中的全局横幅：不管用户停在哪个页都能看到进度 */}
      {["downloading", "installing", "restarting"].includes(updater.phase) && (
        <div className="update-banner" data-no-drag>
          <span className="update-banner-text">
            {updater.phase === "restarting" ? "正在重启…" : updater.phase === "installing" ? "正在安装更新…" : `正在下载更新${updater.latest ? ` v${updater.latest}` : ""}… ${Math.round(updater.progress * 100)}%`}
          </span>
          <div className="update-banner-bar">
            <i style={{ width: `${Math.max(4, Math.round(updater.progress * 100))}%` }} />
          </div>
          <span className="update-banner-hint">完成后将自动重启</span>
        </div>
      )}
    </div>
  );
}

function SidebarAbout({ active, onClick }: { active: boolean; onClick: () => void }) {
  return (
    <button
      className={active ? "sidebar-footer about active" : "sidebar-footer about"}
      onClick={onClick}
    >
      <span className="footer-avatar">
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
          <circle cx="8" cy="8" r="6.5" stroke="currentColor" strokeWidth="1.4" />
          <circle cx="8" cy="5.4" r="0.9" fill="currentColor" />
          <path
            d="M8 7.4v3.4"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
      </span>
      <span>关于</span>
    </button>
  );
}

/* ---- 导航图标（纯展示，inline SVG） ---- */

function IconRemote() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
      <rect x="4.5" y="1.5" width="7" height="13" rx="3.5" stroke="currentColor" strokeWidth="1.4" />
      <circle cx="8" cy="5" r="1" fill="currentColor" />
      <circle cx="8" cy="9" r="1" fill="currentColor" />
    </svg>
  );
}

function IconSliders() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
      <path d="M2 4.5h12M2 11.5h12" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      <circle cx="6" cy="4.5" r="1.8" fill="#eeeff0" stroke="currentColor" strokeWidth="1.4" />
      <circle cx="10.5" cy="11.5" r="1.8" fill="#eeeff0" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}

function IconLayers() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
      <path
        d="M8 2l6 3-6 3-6-3 6-3z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      <path
        d="M2 8.5l6 3 6-3M2 11.5l6 3 6-3"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
    </svg>
  );
}
