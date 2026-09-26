import { useState } from "react";
import { useAppStore } from "../stores/useAppStore";

export default function Presets() {
  const config = useAppStore((s) => s.config);
  const setPreset = useAppStore((s) => s.setPreset);
  const addPreset = useAppStore((s) => s.addPreset);
  const deletePreset = useAppStore((s) => s.deletePreset);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  if (!config) return null;

  const submit = async () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    try {
      await addPreset(trimmed);
      setName("");
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async (target: string) => {
    try {
      await deletePreset(target);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section>
      <h2>Agent Preset</h2>
      <p className="muted">
        Preset = 一组「遥控事件 → 动作」映射。为 Codex、WorkBuddy、Cursor 等分别维护一套，
        在「遥控器映射」页切换后立即生效。
      </p>
      <div className="cards">
        {Object.entries(config.presets).map(([presetName, preset]) => (
          <div
            key={presetName}
            className={presetName === config.preset ? "card preset active" : "card preset"}
          >
            <div className="card-title">
              {presetName}
              {presetName === config.preset && <span className="tag">使用中</span>}
            </div>
            <div className="muted">{Object.keys(preset.mappings).length} 个映射</div>
            <div className="preset-actions">
              {presetName !== config.preset && (
                <button className="btn" onClick={() => setPreset(presetName)}>
                  启用
                </button>
              )}
              {presetName !== config.preset && (
                <button className="btn danger" onClick={() => remove(presetName)}>
                  删除
                </button>
              )}
            </div>
          </div>
        ))}
      </div>
      <div className="card new-preset">
        <input
          placeholder="新 Preset 名称，例如 Cursor"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
        />
        <button className="btn primary" onClick={submit}>
          新建
        </button>
      </div>
      {error && <p className="mapping-result">{error}</p>}
    </section>
  );
}
