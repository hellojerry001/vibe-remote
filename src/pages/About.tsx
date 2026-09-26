import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useUpdater } from "../updater";

const REPO = "https://github.com/hellojerry001/vibe-remote";

/** 关于页：应用信息、版本检查与反馈入口。更新逻辑在 ../updater（全局共用） */
export default function About() {
  // 版本号没回来前先占位，避免闪一下「当前版本 」
  const [version, setVersion] = useState("");
  const {
    phase,
    latest,
    hasNew,
    progress,
    errMsg,
    autoCheck,
    toast,
    setAppVersion,
    setAutoCheck,
    check,
    install,
  } = useUpdater();
  const checking = phase === "checking";
  const updating = phase === "downloading";

  useEffect(() => {
    invoke<string>("get_app_version")
      .then((v) => {
        setVersion(v);
        // 回填给全局更新器：启动检查的兜底比较依赖它
        setAppVersion(v);
      })
      .catch(() => {});
  }, [setAppVersion]);

  const versionSub = hasNew
    ? `发现新版本${latest ? ` v${latest}` : ""}，${updating ? "正在下载安装" : "点击立即更新"}`
    : errMsg
      ? "检查失败，请稍后再试"
      : "已是最新";

  const openInBrowser = (url: string) => {
    void invoke("open_url", { url }).catch(() => {});
  };

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
            <div className="about-row-title">当前版本 {version || "…"}</div>
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
              onClick={() => void (hasNew ? install() : check(false))}
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
            <div className="about-row-sub">启动后静默查一次，发现新版本自动下载安装重启</div>
          </div>
          <input
            type="checkbox"
            aria-label="有新版本时自动检查"
            checked={autoCheck}
            onChange={(e) => setAutoCheck(e.target.checked)}
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
