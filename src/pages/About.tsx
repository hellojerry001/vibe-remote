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
    checked,
    installed,
    setAppVersion,
    setAutoCheck,
    check,
    install,
  } = useUpdater();
  const checking = phase === "checking";
  const updating = phase === "downloading" || phase === "installing" || phase === "restarting";

  useEffect(() => {
    invoke<string>("get_app_version")
      .then((v) => {
        setVersion(v);
        // 回填给全局更新器：启动检查的兜底比较依赖它
        setAppVersion(v);
      })
      .catch(() => {});
  }, [setAppVersion]);

  const versionSub = checking ? "正在检查新版本…"
    : errMsg ? errMsg
    : installed ? "更新已安装，等待重启"
    : hasNew ? `发现新版本${latest ? ` v${latest}` : ""}，${updating ? "正在下载安装" : "点击立即更新"}`
    : checked ? "已是最新" : "尚未检查更新";

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
              <span>{phase === "restarting" ? "正在重启…" : phase === "installing" ? "正在安装…" : `正在下载… ${Math.round(progress * 100)}%`}</span>
            </div>
          ) : (
            <button
              className="btn"
              onClick={() => void ((hasNew || installed) ? install() : check(false))}
              disabled={checking}
            >
              {checking ? "检查中…" : installed ? "重启应用" : hasNew ? "立即更新" : "检查更新"}
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
    </>
  );
}
