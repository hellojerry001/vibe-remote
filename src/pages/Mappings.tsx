import { invoke } from "@tauri-apps/api/core";
import type { TouchpadConfig } from "../types";
import { useEffect, useRef, useState } from "react";
import { useAppStore } from "../stores/useAppStore";
import {
  ACTION_LABELS,
  BUILTIN_ACTIONS,
  EVENT_LABELS,
  KIND_LABELS,
  REMOTE_EVENTS,
} from "../types";
import type { MappingKind, RemoteEvent } from "../types";

const KIND_OPTIONS: MappingKind[] = ["shortcut", "key", "action", "macro", "applescript", "shell"];

/* ---------- 自定义下拉（参考 Chatterfly：灰底触发器 + 白色浮层面板 + 选中 ✓） ---------- */

function Dropdown({
  value,
  options,
  onChange,
}: {
  value: string;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const current = options.find((o) => o.value === value);

  return (
    <div className="select-wrap" ref={wrapRef}>
      <button
        type="button"
        className={open ? "select-btn open" : "select-btn"}
        onClick={() => setOpen(!open)}
      >
        <span>{current?.label ?? value}</span>
        <svg className="select-chevron" width="10" height="6" viewBox="0 0 10 6" fill="none">
          <path d="M1 1l4 4 4-4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {open && (
        <div className="select-pop" role="listbox">
          {options.map((o) => (
            <div
              key={o.value}
              role="option"
              aria-selected={o.value === value}
              className="select-opt"
              onClick={() => {
                onChange(o.value);
                setOpen(false);
              }}
            >
              <span>{o.label}</span>
              {o.value === value && (
                <svg width="12" height="10" viewBox="0 0 12 10" fill="none" className="select-check">
                  <path d="M1 5l3.5 3.5L11 1" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/* ---------- 按键录制（参考 Chatterfly：点一下 → 直接按键盘） ---------- */

/** KeyboardEvent.key → Rust keyboard.rs 认的键名 */
const KEY_NAMES: Record<string, string> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Enter: "return",
  Escape: "escape",
  Tab: "tab",
  " ": "space",
  Backspace: "delete",
  Delete: "forwarddelete",
  Home: "home",
  End: "end",
  PageUp: "pageup",
  PageDown: "pagedown",
};

function keyName(e: KeyboardEvent): string | null {
  if (KEY_NAMES[e.key]) return KEY_NAMES[e.key];
  if (/^F([1-9]|1[0-2])$/.test(e.key)) return e.key.toLowerCase();
  if (e.key.length === 1) return e.key.toLowerCase();
  return null;
}

/** 快捷键修饰符按 Ctrl → Opt → Shift → Cmd 的固定顺序 */
function modTokens(e: KeyboardEvent): string[] {
  const tokens: string[] = [];
  if (e.ctrlKey) tokens.push("Ctrl");
  if (e.altKey) tokens.push("Opt");
  if (e.shiftKey) tokens.push("Shift");
  if (e.metaKey) tokens.push("Cmd");
  return tokens;
}

function KeyCapture({
  kind,
  value,
  onCommit,
}: {
  kind: "key" | "shortcut";
  value: string;
  onCommit: (next: string) => void;
}) {
  const [recording, setRecording] = useState(false);
  const [live, setLive] = useState("");

  const stop = () => {
    setRecording(false);
    setLive("");
  };

  useEffect(() => {
    if (!recording) return;
    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        stop();
        return;
      }
      const name = keyName(e);
      if (!name) {
        // 纯修饰键：显示实时预览
        setLive(kind === "shortcut" ? modTokens(e).join("+") : "");
        return;
      }
      const parts = kind === "shortcut" ? [...modTokens(e), name] : [name];
      onCommit(parts.join("+"));
      stop();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [recording, kind, onCommit]);

  const chips = value ? value.split("+").filter(Boolean) : [];

  return (
    <div
      className={recording ? "key-capture recording" : "key-capture"}
      role="button"
      tabIndex={0}
      onClick={() => !recording && setRecording(true)}
      onKeyDown={(e) => e.key === "Enter" && !recording && setRecording(true)}
      title={recording ? "按 Esc 取消" : "点击后直接按下按键"}
    >
      {recording ? (
        live ? <span className="key-chip">{live}</span> : <span className="key-hint">键盘按下快捷键</span>
        ) : chips.length > 0 ? (
          chips.map((c, i) => <span key={i} className="key-chip">{c}</span>)
        ) : (
          <span className="key-hint">点击设置快捷键</span>
        )}
      {!recording && value && (
        <button
          type="button"
          className="key-clear"
          title="清除"
          onClick={(e) => {
            e.stopPropagation();
            onCommit("");
          }}
        >
          <svg width="9" height="9" viewBox="0 0 9 9" fill="none">
            <path d="M1.5 1.5l6 6m0-6l-6 6" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
          </svg>
        </button>
      )}
    </div>
  );
}

export default function Mappings() {
  const config = useAppStore((s) => s.config);
  const setPreset = useAppStore((s) => s.setPreset);

  if (!config) return null;
  const preset = config.presets[config.preset];

  return (
    <section>
      <div className="page-head">
        <div className="row-head">
          <h2>遥控器映射</h2>
          <Dropdown
            value={config.preset}
            options={Object.keys(config.presets).map((name) => ({ value: name, label: name }))}
            onChange={(v) => setPreset(v)}
          />
        </div>
      </div>

      <TouchpadSettings config={config.touchpad} />

      <TargetAppRow key={config.preset} preset={config.preset} initial={preset?.targetApp ?? ""} />

      <div className="mapping-list">
        {REMOTE_EVENTS.map((event) => {
          const mapping = preset?.mappings[event];
          return (
            <MappingRow
              key={config.preset + event}
              event={event}
              kind={mapping?.kind ?? "action"}
              value={mapping?.value ?? ""}
            />
          );
        })}
      </div>
    </section>
  );
}

/** 目标 App：决定按键最终落到哪个 App。留空 = 当前前台 App。 */
function TargetAppRow({ preset, initial }: { preset: string; initial: string }) {
  const setPresetTarget = useAppStore((s) => s.setPresetTarget);
  const [value, setValue] = useState(initial);
  const [saved, setSaved] = useState<string | null>(null);

  const save = (next: string) => {
    setValue(next);
    setPresetTarget(preset, next)
      .then(() => setSaved(next.trim() ? `已固定发给 ${next.trim()}` : "已改回发给当前前台 App"))
      .catch((e) => setSaved(`✕ ${String(e)}`));
  };

  return (
    <div className="card target-app">
      <div>
        <div className="card-title">目标 App</div>
        <div className="muted">
          按键发给谁。留空 = 发给「当前前台 App」；填了就固定发给它，
          不管你正盯着哪个窗口。优先写 bundle id（如 com.tencent.workbuddy.mac）：
          按名字匹配依赖本地化显示名，不可靠。
        </div>
        {saved && <div className="mapping-result">{saved}</div>}
      </div>
      <div className="stack">
        <input
          value={value}
          placeholder="例如：WorkBuddy 或 com.tencent.workbuddy.mac"
          onChange={(e) => setValue(e.target.value)}
          onBlur={() => value !== initial && save(value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          }}
        />
        {preset === "WorkBuddy" && initial !== "com.tencent.workbuddy.mac" && (
          <button className="btn primary" onClick={() => save("com.tencent.workbuddy.mac")}>
            一键设为 WorkBuddy
          </button>
        )}
      </div>
    </div>
  );
}

function MappingRow({
  event,
  kind,
  value,
}: {
  event: RemoteEvent;
  kind: MappingKind;
  value: string;
}) {
  const setMapping = useAppStore((s) => s.setMapping);
  const testAction = useAppStore((s) => s.testAction);
  const [localKind, setLocalKind] = useState<MappingKind>(kind);
  const [localValue, setLocalValue] = useState(value);
  const [result, setResult] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const realSave = (nextKind: MappingKind, nextValue: string) => {
    const presetName = useAppStore.getState().config?.preset;
    if (!presetName) return;
    setLocalKind(nextKind);
    setLocalValue(nextValue);
    setMapping(presetName, event, nextKind, nextValue).catch((e) => setResult(String(e)));
  };

  const runTest = async () => {
    setBusy(true);
    setResult(null);
    try {
      const r = await testAction(localKind, localValue);
      setResult(`${r.ok ? "✓" : "✕"} ${r.detail}`);
    } catch (e) {
      setResult(`✕ ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mapping-row">
      <div className="mapping-event">{EVENT_LABELS[event]}</div>
      <Dropdown
        value={localKind}
        options={KIND_OPTIONS.map((k) => ({ value: k, label: KIND_LABELS[k] }))}
        onChange={(v) => realSave(v as MappingKind, localValue)}
      />

      {localKind === "action" ? (
        <div className="mapping-value">
          <Dropdown
            value={localValue}
            options={BUILTIN_ACTIONS.map((a) => ({ value: a, label: ACTION_LABELS[a] }))}
            onChange={(v) => realSave(localKind, v)}
          />
        </div>
      ) : localKind === "applescript" || localKind === "shell" ? (
        <div className="mapping-value">
          <textarea
            rows={2}
            value={localValue}
            placeholder={localKind === "shell" ? "例如：open -a Terminal" : "tell application …"}
            onChange={(e) => setLocalValue(e.target.value)}
            onBlur={() => realSave(localKind, localValue)}
          />
        </div>
      ) : localKind === "key" || localKind === "shortcut" ? (
        <div className="mapping-value">
          <KeyCapture
            kind={localKind}
            value={localValue}
            onCommit={(next) => realSave(localKind, next)}
          />
        </div>
      ) : (
        <div className="mapping-value">
          <input
            value={localValue}
            placeholder="Cmd+D|wait:1500|Cmd+Down|Cmd+Right"
            onChange={(e) => setLocalValue(e.target.value)}
            onBlur={() => realSave(localKind, localValue)}
          />
        </div>
      )}

      <button className="btn" onClick={runTest} disabled={busy || localValue === ""}>
        测试
      </button>
      {result && <div className="mapping-result">{result}</div>}
    </div>
  );
}

function TouchpadSettings({ config }: { config: TouchpadConfig }) {
  const loadConfig = useAppStore((s) => s.loadConfig);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [pct, setPct] = useState(((config.gain - 0.1) / 29.9) * 100);
  const save = async (next: TouchpadConfig) => {
    setBusy(true);
    setError("");
    try {
      await invoke("set_touchpad", { config: next });
      await loadConfig();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  };
  return (
    <div className="card settings-card">
      <div className="settings-head">
        <div className="card-title">触控板控制鼠标</div>
        <p className="muted">
          单指滑动移动光标，轻点左键点击。需要辅助功能权限，修改即时保存生效；物理按键继续使用下方映射。
        </p>
      </div>
      <div
        className="settings-rows"
        style={busy ? { opacity: 0.5, pointerEvents: "none" } : undefined}
      >
        <div className="settings-row">
          <div>
            <div className="settings-row-title">启用鼠标控制</div>
            <div className="muted">关闭后触控板不再移动光标</div>
          </div>
          <input
            type="checkbox"
            aria-label="启用鼠标控制"
            checked={config.enabled}
            onChange={(e) => void save({ ...config, enabled: e.target.checked })}
          />
        </div>
        <div className="settings-row">
          <div>
            <div className="settings-row-title">轻点点击</div>
            <div className="muted">轻触触摸板即左键点击</div>
          </div>
          <input
            type="checkbox"
            aria-label="轻点点击"
            checked={config.tapClick}
            onChange={(e) => void save({ ...config, tapClick: e.target.checked })}
          />
        </div>
        <div className="settings-row">
          <div>
            <div className="settings-row-title">灵敏度</div>
            <div className="muted">光标移动速度 {config.gain.toFixed(1)} ×</div>
          </div>
          <div className="slider-block">
            <input
              aria-label="鼠标灵敏度"
              type="range"
              min="0.1"
              max="30"
              step="0.1"
              defaultValue={config.gain}
              key={config.gain}
              className="settings-slider"
              style={{ "--pct": `${pct}%` } as React.CSSProperties}
              onInput={(e) => {
                const el = e.currentTarget;
                setPct(((Number(el.value) - 0.1) / 29.9) * 100);
              }}
              onPointerUp={(e) => { const gain = Number(e.currentTarget.value); if (gain !== config.gain) void save({ ...config, gain }); }}
              onKeyUp={(e) => { const gain = Number(e.currentTarget.value); if (gain !== config.gain) void save({ ...config, gain }); }}
            />
            <div className="slider-ticks" aria-hidden="true">
              {Array.from({ length: 7 }, (_, i) => (
                <span
                  key={i}
                  style={{
                    left: `${(i * 100) / 6}%`,
                    background: (i * 100) / 6 <= pct ? "#3a3a3e" : "#dcdcdf",
                  }}
                />
              ))}
            </div>
            <div className="slider-labels" aria-hidden="true">
              <span className="slider-label-min">慢</span>
              <span className="slider-label-mid">默认</span>
              <span className="slider-label-max">快</span>
            </div>
          </div>
        </div>
      </div>
      {error && <p role="alert">保存失败：{error}</p>}
    </div>
  );
}
