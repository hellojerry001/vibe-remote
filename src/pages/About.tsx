import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { check as checkUpdater } from "@tauri-apps/plugin-updater";

const REPO = "https://github.com/hellojerry001/vibe-remote";
const AUTO_CHECK_KEY = "viberemote.autoCheck";
/** 与 tauri.conf.json 的 updater.endpoints 保持一致 */
const UPDATE_JSON = "https://raw.githubusercontent.com/hellojerry001/vibe-remote/main/update.json";

/** 比较两个 x.y.z 版本号，>0 表示 a 更新 */
function cmpVersion(a: string, b: string): number {
  const pa = a.replace(/^v/i, "").split(".").map((n) => parseInt(n, 10) || 0);
  const pb = b.replace(/^v/i, "").split(".").map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < 3; i += 1) {
    if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) - (pb[i] ?? 0);
  }
  return 0;
}

/** 关于页：应用信息、版本检查与反馈入口（参考 VibeButler 关于页布局） */
export default function About() {
  // 先空着：版本号没回来之前不跑自动检查，避免 fallback 用旧版本号误判「发现新版本」
  const [version, setVersion] = useState("");
  const [latest, setLatest] = useState<string | null>(null);
  const [hasNew, setHasNew] = useState(false);
  const [checking, setChecking] = useState(false);
  const [checkFailed, setCheckFailed] = useState(false);
  const [autoCheck, setAutoCheck] = useState(
    () => localStorage.getItem(AUTO_CHECK_KEY) !== "0",
  );
  const [updating, setUpdating] = useState(false);
  const [progress, setProgress] = useState(0);
  const [toast, setToast] = useState<string | null>(null);
  const toastTimerRef = useRef<number | null>(null);
  const totalRef = useRef(0);
  const downloadedRef = useRef(0);
  // 自动检查只在本次会话跑一次
  const autoRanRef = useRef(false);

  useEffect(() => {
    invoke<string>("get_app_version")
      .then(setVersion)
      .catch(() => {});
  }, []);

  /**
   * 内置更新器失败时的兜底：直接读同一份 update.json。
   * 返回 { version, hasUpdate }；"none" = 仓库还没有 update.json（视为已是最新）；null = 网络失败。
   */
  const fetchLatest = async (): Promise<{ version: string; hasUpdate: boolean } | "none" | null> => {
    try {
      const res = await fetch(UPDATE_JSON, { cache: "no-store" });
      if (res.status === 404) return "none";
      if (!res.ok) return null;
      const j = (await res.json()) as { version?: unknown };
      const v = typeof j.version === "string" ? j.version.replace(/^v/i, "") : null;
      if (!v) return null;
      return { version: v, hasUpdate: cmpVersion(v, version) > 0 };
    } catch {
      return null;
    }
  };

  const runCheck = async (silent: boolean) => {
    setChecking(true);
    setCheckFailed(false);
    let manualNoUpdate = false;
    let manualFailed = false;
    try {
      const update = await checkUpdater();
      const isNew = !!update;
      setLatest(update?.version ?? null);
      setHasNew(isNew);
      if (!isNew && !silent) manualNoUpdate = true;
      // 自动检查发现新版本 → 直接下载并安装，不用用户点
      if (isNew && silent && !updating) {
        window.setTimeout(() => void doUpdate(), 300);
      }
    } catch {
      // 内置更新器失败时退回直接读 update.json，保证这条路始终可用
      const tag = await fetchLatest();
      if (tag === "none") {
        setHasNew(false);
        if (!silent) manualNoUpdate = true;
      } else if (tag) {
        setLatest(tag.version);
        setHasNew(tag.hasUpdate);
        if (!silent) manualNoUpdate = !tag.hasUpdate;
      } else {
        setCheckFailed(true);
        if (!silent) manualFailed = true;
      }
    } finally {
      setChecking(false);
    }
    if (manualNoUpdate) showToast("当前已是最新版本");
    else if (manualFailed) showToast("检查失败，请稍后再试");
  };

  /** 轻提示：参考 Chatterfly 的「✓ 当前已是最新版本」胶囊 */
  const showToast = (msg: string) => {
    setToast(msg);
    if (toastTimerRef.current) window.clearTimeout(toastTimerRef.current);
    // 失败信息比较长，多留几秒让人看清原因
    toastTimerRef.current = window.setTimeout(() => setToast(null), 8000);
  };

  /** 真正下载安装包 + 替换应用 + 重启，而不是跳到浏览器 */
  const doUpdate = async () => {
    if (updating) return;
    setUpdating(true);
    setProgress(0);
    try {
      const update = await checkUpdater();
      if (!update) {
        setHasNew(false);
        return;
      }
      totalRef.current = 0;
      downloadedRef.current = 0;
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") {
          totalRef.current = e.data.contentLength ?? 0;
        } else if (e.event === "Progress") {
          downloadedRef.current += e.data.chunkLength;
          if (totalRef.current > 0) {
            setProgress(Math.min(1, downloadedRef.current / totalRef.current));
          }
        }
      });
      // macOS 安装后需要重启才会跑新版本
      await invoke("restart_app");
    } catch (e) {
      // 最常见的失败是「更新包没签名 / 公钥不匹配」：给明确提示，别只回退下载页
      const raw = e instanceof Error ? e.message : String(e);
      const msg = raw.toLowerCase();
      setCheckFailed(true);
      const unsigned = msg.includes("signature") || msg.includes("verify") || msg.includes("not signed");
      showToast(
        (unsigned ? "更新包未通过签名校验：" : "更新失败：") + raw.replace(/\s+/g, " ").slice(0, 90),
      );
      openInBrowser(`${REPO}/releases`);
    } finally {
      setUpdating(false);
    }
  };

  // 启动后静默查一次（开关打开时）
  useEffect(() => {
    if (autoCheck && !autoRanRef.current && version) {
      autoRanRef.current = true;
      void runCheck(true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoCheck, version]);

  const openInBrowser = (url: string) => {
    void invoke("open_url", { url }).catch(() => {});
  };

  const toggleAuto = (on: boolean) => {
    setAutoCheck(on);
    localStorage.setItem(AUTO_CHECK_KEY, on ? "1" : "0");
    if (on && !autoRanRef.current) {
      autoRanRef.current = true;
      void runCheck(true);
    }
  };

  const versionSub = hasNew
    ? `发现新版本${latest ? ` v${latest}` : ""}，正在下载安装`
    : checkFailed
      ? "检查失败，请稍后再试"
      : "已是最新";

  return (
    <>
      <div className="page-head">
        <h2>关于</h2>
        <p className="subtitle">版本信息与更新</p>
      </div>

      <div className="about-hero">
        <img className="about-icon" src="/app-icon.png" alt="VibeRemote" draggable={false} />
        <div className="about-name">VibeRemote</div>
        <div className="about-desc">用 Siri Remote 控制 AI 编码代理</div>
      </div>

      <div className="about-card">
        <div className="about-row">
          <div>
            <div className="about-row-title">当前版本 {version}</div>
            <div className={`about-row-sub ${hasNew ? "about-new" : ""}`}>{versionSub}</div>
          </div>
          {updating ? (
            <div className="about-progress">
              <div className="about-progress-bar">
                <i style={{ width: `${Math.round(progress * 100)}%` }} />
              </div>
              <span>正在下载并安装… {Math.round(progress * 100)}%</span>
            </div>
          ) : (
            <button
              className="btn"
              onClick={hasNew ? () => void doUpdate() : () => void runCheck(false)}
              disabled={checking}
            >
              {checking ? "检查中…" : hasNew ? "立即更新" : "检查更新"}
            </button>
          )}
        </div>
        <div className="about-divider" />
        <div className="about-row">
          <div>
            <div className="about-row-title">有新版本时自动检查</div>
            <div className="about-row-sub">启动后静默查一次，只在真的发现新版本时才提示</div>
          </div>
          <input
            type="checkbox"
            aria-label="有新版本时自动检查"
            checked={autoCheck}
            onChange={(e) => toggleAuto(e.target.checked)}
          />
        </div>
      </div>

      <div className="about-card">
        <button className="about-link-row" onClick={() => openInBrowser(`${REPO}/issues`)}>
          <div>
            <div className="about-row-title">反馈问题或建议</div>
            <div className="about-row-sub">在仓库里开一个 issue</div>
          </div>
          <svg className="about-arrow" width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path
              d="M5 11L11 5M11 5H6.5M11 5v4.5"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>

      <div className="about-foot">
        <span>GitHub · hellojerry001/vibe-remote</span>
        <span>检查更新只访问这个仓库</span>
      </div>

      {toast && (
        <div className="about-toast" data-no-drag>
          <span className="about-toast-icon">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <circle cx="6" cy="6" r="6" fill="#34a853" />
              <path
                d="M3.6 6.2l1.6 1.6 3-3.4"
                stroke="#fff"
                strokeWidth="1.4"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </span>
          {toast}
        </div>
      )}
    </>
  );
}
