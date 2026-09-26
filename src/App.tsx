import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import Mappings from "./pages/Mappings";
import Presets from "./pages/Presets";
import Remote from "./pages/Remote";
import { useAppStore } from "./stores/useAppStore";
import type { ActionResult, BtDevice, ConnectionState, HidEvent, Status } from "./types";

const TABS = [
  { id: "remote", label: "设备状态" },
  { id: "mappings", label: "遥控器映射" },
  { id: "presets", label: "Agent Preset" },
] as const;

type TabId = (typeof TABS)[number]["id"];

export default function App() {
  const [tab, setTab] = useState<TabId>("remote");
  const loadConfig = useAppStore((s) => s.loadConfig);
  const refreshStatus = useAppStore((s) => s.refreshStatus);
  const setStatus = useAppStore((s) => s.setStatus);
  const setLastHid = useAppStore((s) => s.setLastHid);
  const pushEvent = useAppStore((s) => s.pushEvent);
  const setConnection = useAppStore((s) => s.setConnection);
  const addFoundDevice = useAppStore((s) => s.addFoundDevice);
  const setScanNote = useAppStore((s) => s.setScanNote);
  const refreshConnection = useAppStore((s) => s.refreshConnection);

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

  return (
    <div className="layout">
      <aside className="sidebar">
        <div className="brand">Web Coding</div>
        <nav className="nav">
          {TABS.map((t) => (
            <button
              key={t.id}
              className={tab === t.id ? "nav-item active" : "nav-item"}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
        <SidebarStatus />
      </aside>
      <main className="content">
        {tab === "remote" && <Remote />}
        {tab === "mappings" && <Mappings />}
        {tab === "presets" && <Presets />}
      </main>
    </div>
  );
}

function SidebarStatus() {
  const status = useAppStore((s) => s.status);
  return (
    <div className="sidebar-footer">
      <span className={status?.tvConnected ? "dot on" : "dot"} />
      <span>TV {status?.tvConnected ? "已连接" : "未连接"}</span>
      <span className="muted">:{status?.port || "—"}</span>
    </div>
  );
}
