import { invoke } from "@tauri-apps/api/core";
import type { TouchpadConfig } from "../types";
import { useState } from "react";
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

const COMMON_KEYS = ["return", "escape", "up", "down", "left", "right", "tab", "space", "delete"];

export default function Mappings() {
  const config = useAppStore((s) => s.config);
  const setPreset = useAppStore((s) => s.setPreset);

  if (!config) return null;
  const preset = config.presets[config.preset];

  return (
    <section>
      <div className="row-head">
        <h2>遥控器映射</h2>
        <select value={config.preset} onChange={(e) => setPreset(e.target.value)}>
          {Object.keys(config.presets).map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </div>
      <p className="muted">
        当前 Preset「{config.preset}」，修改即时保存；「测试」会真实执行一次该动作。
      </p>

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
          不管你正盯着哪个窗口。**优先写 bundle id**（如 com.tencent.workbuddy.mac）：
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
      <select value={localKind} onChange={(e) => realSave(e.target.value as MappingKind, localValue)}>
        {KIND_OPTIONS.map((k) => (
          <option key={k} value={k}>
            {KIND_LABELS[k]}
          </option>
        ))}
      </select>

      {localKind === "action" ? (
        <select value={localValue} onChange={(e) => realSave(localKind, e.target.value)}>
          {BUILTIN_ACTIONS.map((a) => (
            <option key={a} value={a}>
              {ACTION_LABELS[a]}
            </option>
          ))}
        </select>
      ) : localKind === "applescript" || localKind === "shell" ? (
        <textarea
          rows={2}
          value={localValue}
          placeholder={localKind === "shell" ? "例如：open -a Terminal" : "tell application …"}
          onChange={(e) => setLocalValue(e.target.value)}
          onBlur={() => realSave(localKind, localValue)}
        />
      ) : (
        <input
          value={localValue}
          list={localKind === "key" ? "common-keys" : undefined}
          placeholder={localKind === "shortcut" ? "Cmd+Enter" : "escape"}
          onChange={(e) => setLocalValue(e.target.value)}
          onBlur={() => realSave(localKind, localValue)}
        />
      )}

      <button className="btn" onClick={runTest} disabled={busy || localValue === ""}>
        测试
      </button>
      {result && <div className="mapping-result">{result}</div>}
      <datalist id="common-keys">
        {COMMON_KEYS.map((k) => (
          <option key={k} value={k} />
        ))}
      </datalist>
    </div>
  );
}

function TouchpadSettings({ config }: { config: TouchpadConfig }) {
  const loadConfig = useAppStore((s) => s.loadConfig);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const save = async (next: TouchpadConfig) => {
    setBusy(true);
    setError("");
    try {
      await invoke("set_touchpad", { config: next });
      await loadConfig();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  };
  return <div className="card">
    <div className="card-title">触控板控制鼠标</div>
    <p className="muted">单指滑动移动光标，轻点左键点击。需要辅助功能权限，修改即时保存生效。物理按键继续使用下方映射。</p>
    <fieldset disabled={busy} style={{ border: 0, padding: 0, display: "flex", gap: 20, flexWrap: "wrap" }}>
      <label><input type="checkbox" checked={config.enabled} onChange={(e) => void save({ ...config, enabled: e.target.checked })} /> 启用鼠标控制</label>
      <label><input type="checkbox" checked={config.tapClick} onChange={(e) => void save({ ...config, tapClick: e.target.checked })} /> 轻点点击</label>
      <label>灵敏度 {config.gain.toFixed(1)} × <input aria-label="鼠标灵敏度" type="range" min="0.1" max="30" step="0.1" defaultValue={config.gain} key={config.gain}
        onPointerUp={(e) => { const gain = Number(e.currentTarget.value); if (gain !== config.gain) void save({ ...config, gain }); }}
        onKeyUp={(e) => { const gain = Number(e.currentTarget.value); if (gain !== config.gain) void save({ ...config, gain }); }} /></label>
    </fieldset>
    {error && <p role="alert">保存失败：{error}</p>}
  </div>;
}
