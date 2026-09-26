import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../stores/useAppStore";
import type { BtDevice, ConnectionState, LayerLevel, PermissionLevel, SystemStatus } from "../types";

const PERMISSION_TEXT: Record<PermissionLevel, string> = {
  granted: "Granted",
  denied: "Denied",
  unknown: "Unknown",
};

export default function Remote() {
  const status = useAppStore((s) => s.status);
  const events = useAppStore((s) => s.events);
  const config = useAppStore((s) => s.config);
  const lastHid = useAppStore((s) => s.lastHid);
  const refreshStatus = useAppStore((s) => s.refreshStatus);
  const [sys, setSys] = useState<SystemStatus | null>(null);
  const connection = useAppStore((s) => s.connection);

  // 权限与 HID 状态一律向 Rust 要真实值，前端不猜
  const refreshSystem = useCallback(async () => {
    try {
      setSys(await invoke<SystemStatus>("get_system_status"));
    } catch {
      /* 状态读取失败不影响主流程 */
    }
  }, []);
  const refreshConnection = useAppStore((s) => s.refreshConnection);

  // 连接状态机走的是事件推送，这里只补一次拉取，避免刷新前是空白
  useEffect(() => {
    void refreshConnection();
  }, [refreshConnection]);

  useEffect(() => {
    refreshStatus();
    refreshSystem();
    const timer = setInterval(() => {
      refreshStatus();
      refreshSystem();
    }, 3000);
    return () => clearInterval(timer);
  }, [refreshStatus, refreshSystem]);

  return (
    <section>
      <div className="page-head">
        <h2>设备状态</h2>
      </div>
      <div className="cards">
        <StatusCard
          title="Siri Remote"
          value={siriValue(connection, status?.connectedDevices ?? 0)}
        />
        <StatusCard
          title="Mac 服务"
          value={status?.running ? `运行中 · 端口 ${status.port}` : "未启动"}
        />
        <StatusCard title="前台 App" value={status?.frontApp || "—"} />
      </div>

      <SetupWizard connection={connection} />

      <div className="card">
        <div className="card-title">中央触控区域 → 鼠标</div>
        <p className="muted">
          {!config?.touchpad.enabled ? "鼠标控制已关闭，请到遥控器映射中启用。"
            : status?.touchpad.error ? `触控通道打开失败（${status.touchpad.error}）`
            : !status?.touchpad.devices ? "等待遥控器触控区域连接"
            : !status.accessibility ? "已连接触控区域，需要授权辅助功能才能移动鼠标"
            : `触控区域已连接 · 收到 ${status.touchpad.frames} 帧 · 已发送 ${status.touchpad.pointerMoves} 次鼠标移动`}
        </p>
        <div className="muted">在中间圆形区域滑动，不用按下。若帧数一直为 0，尚未收到触摸数据。</div>
      </div>

      <div className="card permission">
        <div>
          <div className="card-title">辅助功能权限</div>
          <div className="muted">
            状态：{PERMISSION_TEXT[sys?.accessibility ?? "unknown"]}
            {" · "}
            {sys?.accessibility === "granted"
              ? "键盘模拟与按钮点击可用"
              : "键盘模拟与按钮点击不会生效"}
          </div>
        </div>
        <button className="btn primary" onClick={() => invoke("request_accessibility_settings")}>
          去授权
        </button>
      </div>

      <div className="card permission">
        <div>
          <div className="card-title">输入监控权限</div>
          <div className="muted">
            状态：{PERMISSION_TEXT[sys?.inputMonitoring ?? "unknown"]}
            {" · "}
            {sys?.inputMonitoring === "granted"
              ? "A2854 按键可以上报"
              : sys?.inputMonitoring === "denied"
                ? "Siri Remote 的按键读不到"
                : "尚未请求过授权"}
          </div>
          {sys?.inputMonitoring === "granted" && !sys.a2854Matched && (
            <div className="mapping-result">
              输入监控已授权，但没匹配到 Apple 遥控器。白名单：
              {sys.productIdWhitelist.join(" / ")}（VID {sys.vendorId}）。
              先在系统蓝牙里确认遥控器是「已连接」，再回到本页。
            </div>
          )}
          {sys?.inputMonitoring === "granted" && sys.a2854Matched && !sys.hidManagerOpen && (
            <div className="mapping-result">
              已匹配到遥控器（PID {sys.productId}），但 HID Manager 没打开。
              权限若是刚改的，请彻底退出并重新启动 VibeRemote。
            </div>
          )}
          {status?.hidError && <DiagError detail={status.hidError} />}
        </div>
        <button className="btn primary" onClick={() => invoke("open_input_monitoring_settings")}>
          系统设置
        </button>
      </div>

      <h3>设备诊断</h3>
      <p className="muted">下列状态全部由 Rust / macOS Native 层真实读取，前端不做推断。</p>
      <div className="card">
        <div className="hid-grid">
          <span className="muted">Input Monitoring</span>
          <span>{PERMISSION_TEXT[sys?.inputMonitoring ?? "unknown"]}</span>
          <span className="muted">Accessibility</span>
          <span>{PERMISSION_TEXT[sys?.accessibility ?? "unknown"]}</span>
          <span className="muted">A2854 Matched</span>
          <span>{sys?.a2854Matched ? "Yes" : "No"}</span>
          <span className="muted">Matched Device Count</span>
          <span>{sys?.matchedDeviceCount ?? 0}</span>
          <span className="muted">Vendor ID</span>
          <span>{sys?.vendorId ?? "—"}</span>
          <span className="muted">Product ID（实际匹配到的）</span>
          <span className={sys && !sys.productId ? "fail" : ""}>
            {sys?.productId ?? "未匹配到遥控器"}
          </span>
          <span className="muted">Product ID 白名单</span>
          <span>{sys?.productIdWhitelist.join(" / ") ?? "—"}</span>
          <span className="muted">IOHIDManager</span>
          <span>{sys?.hidManagerOpen ? "Open" : "Failed"}</span>
          <span className="muted">Input Callback</span>
          <span>{sys?.inputCallbackActive ? "Active" : "Inactive"}</span>
          <span className="muted">Last HID Event</span>
          <span>
            {sys?.lastHidEvent
              ? `0x${sys.lastHidEvent.usagePage.toString(16)} / 0x${sys.lastHidEvent.usage.toString(16)} / ${sys.lastHidEvent.value}`
              : "—"}
          </span>
          <span className="muted">Bundle ID</span>
          <span>{sys?.bundleId ?? "—"}</span>
          <span className="muted">App Path</span>
          <span>{sys?.appPath ?? "—"}</span>
        </div>
      </div>

      <h3>最近指令</h3>
      {events.length === 0 ? (
        <p className="muted">
          还没有收到遥控事件。按下 Siri Remote 的按键，或打开 tvOS App（需与 Mac 同一局域网）。
        </p>
      ) : (
        <table className="events">
          <thead>
            <tr>
              <th>时间</th>
              <th>按键</th>
              <th>动作</th>
              <th>结果</th>
            </tr>
          </thead>
          <tbody>
            {events.map((e, i) => (
              <tr key={i}>
                <td className="muted">{e.ts}</td>
                <td>{e.event}</td>
                <td className="muted">
                  {e.kind}
                  {e.value ? ` · ${e.value}` : ""}
                </td>
                <td className={e.ok ? "" : "fail"}>
                  {e.ok ? "✓" : "✕"} {e.detail}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <h3>HID 原始事件</h3>
      <p className="muted">
        任何按键（含未映射的音量键）都会实时显示在这里，可确认物理键是否已被 Mac 读到。
      </p>
      <div className="card">
        {lastHid === null ? (
          <span className="muted">按一下 Siri Remote 的任意按键试试</span>
        ) : (
          <div className="hid-grid">
            <span className="muted">逻辑名</span>
            <span>{lastHid.event}</span>
            <span className="muted">Usage page</span>
            <span>0x{lastHid.usagePage.toString(16)}</span>
            <span className="muted">Usage</span>
            <span>0x{lastHid.usage.toString(16)}</span>
            <span className="muted">状态</span>
            <span>{lastHid.pressed ? "按下" : "抬起"}</span>
          </div>
        )}
      </div>
      <p className="muted">
        当前 Preset「{config?.preset}」，可在「遥控器映射」页调整每个按键的行为。
      </p>
    </section>
  );
}

/** 顶部三格卡片里的 Siri Remote 一句话状态 */
function siriValue(connection: ConnectionState | null, devices: number): string {
  const m = connection?.machine;
  if (m === "hid_ready") return "已连接 · 按键可用";
  if (m === "connected") return "已连接";
  if (m === "pairing") return "配对中";
  if (m === "found") return "已发现，待配对";
  if (m === "scanning") return "搜索中";
  if (m === "failed") return "连接失败";
  if (m === "paired" || m === "connecting") return "等待连接";
  return devices > 0 ? "已连接" : "未连接";
}

const LAYER_TEXT: Record<LayerLevel, string> = {
  off: "未参与",
  pending: "进行中",
  working: "正常",
  error: "异常",
};

/** 三层独立读数：蓝牙配对 / 链路连接 / HID 输入，互不替代 */
function LayerRow({ name, level, detail }: { name: string; level: LayerLevel; detail: string }) {
  return (
    <div className="layer-row">
      <span className={`dot ${level === "working" ? "on" : level === "error" ? "bad" : ""}`} />
      <span className="layer-name">{name}</span>
      <span className="layer-detail">{detail}</span>
      <span className={`layer-tag ${level}`}>{LAYER_TEXT[level]}</span>
    </div>
  );
}

function ThreeLayers({ c }: { c: ConnectionState }) {
  return (
    <div className="layers">
      <LayerRow name="Bluetooth" level={c.bluetooth} detail={c.bluetoothDetail} />
      <LayerRow name="Connection" level={c.connection} detail={c.connectionDetail} />
      <LayerRow name="HID Input" level={c.hid} detail={c.hidDetail} />
    </div>
  );
}

/** 首次使用的三步向导：连遥控器 → 授权输入监控 → 按确认键验证 */
function SetupWizard({ connection }: { connection: ConnectionState | null }) {
  const found = useAppStore((s) => s.found);
  const scanNote = useAppStore((s) => s.scanNote);
  const startScan = useAppStore((s) => s.startScan);
  const stopScan = useAppStore((s) => s.stopScan);

  if (!connection) return null;
  const m = connection.machine;
  const siriFound = found.filter((d) => d.isSiriRemote);

  return (
    <div className="card wizard">
      <div className="wizard-head">
        <div>
          <div className="card-title">Siri Remote 连接</div>
          <div className="muted">
            {connection.step ? `第 ${connection.step} / 3 步 · ` : ""}
            {connection.machineLabel}
          </div>
        </div>
        <div className="pips">
          {[1, 2, 3].map((n) => (
            <span key={n} className={`pip ${(connection.step ?? 0) >= n ? (m === "failed" ? "bad" : "on") : ""}`}>
              {n}
            </span>
          ))}
        </div>
      </div>

      <ThreeLayers c={connection} />

      <div className="wizard-body">
        {(m === "idle" || m === "instructions") && (
          <>
            <ol className="hint-list">
              <li>
                让遥控器进入配对模式：同时按住 <kbd>返回</kbd> + <kbd>音量+</kbd>，约 5 秒，
                白色指示灯开始闪烁。
              </li>
              <li>回到这台 Mac，点下面的「开始搜索」。</li>
            </ol>
            <div className="row">
              <button className="btn primary" onClick={() => void startScan()}>
                开始搜索
              </button>
              <FallbackButton />
            </div>
          </>
        )}

        {m === "scanning" && (
          <>
            <p className="muted">正在搜索这台 Mac 已知蓝牙设备，持续约 10 秒…</p>
            {scanNote && <p className="mapping-result">{scanNote}</p>}
            {found.length > 0 && <DeviceList devices={found} />}
            <div className="row">
              <button className="btn" onClick={() => void stopScan()}>
                停止搜索
              </button>
              <FallbackButton />
            </div>
          </>
        )}

        {m === "found" && (
          <>
            {siriFound.length > 0 ? (
              <p className="muted">发现 Siri Remote，在系统蓝牙里点它完成配对即可：</p>
            ) : (
              <p className="muted">发现设备，但没有识别到 Siri Remote：</p>
            )}
            {scanNote && <p className="mapping-result">{scanNote}</p>}
            <DeviceList devices={found} />
            <div className="row">
              <FallbackButton primary />
              <button className="btn" onClick={() => void startScan()}>
                重新搜索
              </button>
            </div>
            <p className="muted">
              配对必须在系统蓝牙里完成：macOS 会直接终止尝试调用蓝牙 API 的进程，
              所以这里只能读取设备列表，没法代你按下遥控器上的配对按钮。
            </p>
          </>
        )}

        {m === "pairing" && (
          <>
            <p className="muted">
              A2854 已经在系统蓝牙里了，链路还没通上。请在系统蓝牙设置里点一下它右侧的「连接」。
            </p>
            {scanNote && <p className="mapping-result">{scanNote}</p>}
            <DeviceList devices={siriFound.length > 0 ? siriFound : found} />
            <div className="row">
              <FallbackButton primary />
              <button className="btn" onClick={() => void startScan()}>
                重新搜索
              </button>
            </div>
          </>
        )}

        {(m === "paired" || m === "connecting") && (
          <>
            <p className="muted">
              {m === "paired"
                ? "蓝牙配对已完成，等待链路连上。"
                : "链路已建立，正在接入 HID 输入。"}
            </p>
            <div className="row">
              <button className="btn" onClick={() => void startScan()}>
                重新搜索
              </button>
              <FallbackButton />
            </div>
          </>
        )}

        {m === "connected" && (
          <>
            <p className="muted">
              已连接。现在按一下遥控器中间的<b>确认键</b>，下方「HID 原始事件」应立即出现一行数据。
            </p>
            <div className="row">
              <button className="btn" onClick={() => void startScan()}>
                重新搜索
              </button>
              <FallbackButton />
            </div>
          </>
        )}

        {m === "hid_ready" && (
          <div className="ready">
            <div className="ready-title">VibeRemote Ready</div>
            <div className="ready-rows">
              <span>Siri Remote</span>
              <span className="ok">● Connected</span>
              <span>Input Monitor</span>
              <span className="ok">● Granted</span>
              <span>Agent</span>
              <span className="ok">WorkBuddy</span>
            </div>
          </div>
        )}

        {m === "failed" && (
          <>
            <pre className="diag-error">
              {`无法读取 Siri Remote 输入
可能原因：
1. 输入监控权限未开启
2. 权限开启后 App 尚未重启
3. Siri Remote 未连接
4. A2854 HID 设备未匹配
5. HID Manager 启动失败
6. 链路在系统蓝牙里被断开过

原始信息：${connection.error ?? "—"}`}
            </pre>
            <div className="row">
              <button className="btn primary" onClick={() => void startScan()}>
                重新搜索
              </button>
              <FallbackButton />
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** 系统蓝牙设置入口。在「发现设备 / 配对中」这类状态下它就是主操作，其余时候是次操作。 */
function FallbackButton({ primary = false }: { primary?: boolean }) {
  return (
    <button
      className={primary ? "btn primary" : "btn"}
      onClick={() => void invoke("open_bluetooth_settings")}
    >
      打开系统蓝牙设置
    </button>
  );
}

function DeviceList({ devices }: { devices: BtDevice[] }) {
  if (devices.length === 0) {
    return <p className="muted">还没有发现设备。</p>;
  }
  return (
    <ul className="device-list">
      {devices.map((d) => (
        <li key={d.address} className={d.isSiriRemote ? "is-siri" : ""}>
          <span className="dev-name">{d.name || d.address}</span>
          <span className="muted dev-meta">
            {d.isSiriRemote && <span className="ok">Siri Remote · </span>}
            {d.connected ? "已连接" : "已配对，未连接"}
            {d.vendorId !== null && (
              <span>
                {" · "}
                {d.vendorId.toString(16).toUpperCase().padStart(4, "0")}:
                {(d.productId ?? 0).toString(16).toUpperCase().padStart(4, "0")}
              </span>
            )}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** 统一的多因提示，替代原来一句「输入监控未授权」 */
function DiagError({ detail }: { detail: string }) {
  return (
    <pre className="diag-error">
      {`无法读取 Siri Remote 输入
可能原因：
1. 输入监控权限未开启
2. 权限开启后 App 尚未重启
3. Siri Remote 未连接
4. A2854 HID 设备未匹配
5. HID Manager 启动失败

原始信息：${detail}`}
    </pre>
  );
}

function StatusCard({ title, value }: { title: string; value: string }) {
  return (
    <div className="card">
      <div className="card-title">{title}</div>
      <div className="card-value">{value}</div>
    </div>
  );
}
