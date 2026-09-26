//! WebSocket 服务器 + Bonjour（mDNS）广播。
//!
//! 链路：tvOS App 经 Bonjour 发现本服务 → `ws://<host>:<port>/ws` → 发送 `{"event":"center"}` JSON 文本帧
//! → 本模块解析并分派 MappingEngine → 结果通过 Tauri 事件推给前端。

use std::sync::atomic::Ordering;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State as AxumState;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::mapping::engine::REMOTE_EVENTS;
use crate::AppState;

/// 启动 WebSocket 服务（随机端口 + Bonjour 注册），整个生命周期挂在后台
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let router = Router::new()
            .route("/ws", get(ws_handler))
            .with_state(app.clone());

        let listener = match tokio::net::TcpListener::bind("0.0.0.0:0").await {
            Ok(l) => l,
            Err(e) => {
                let _ = app.emit(
                    "server-status",
                    json!({ "running": false, "port": 0, "tvConnected": false, "error": e.to_string() }),
                );
                return;
            }
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);

        if let Some(state) = app.try_state::<AppState>() {
            state.server_port.store(port, Ordering::Relaxed);
        }

        match register_mdns(port) {
            Ok(daemon) => {
                if let Some(state) = app.try_state::<AppState>() {
                    *state.mdns.lock().unwrap() = Some(daemon);
                }
            }
            Err(e) => eprintln!("[viberemote] Bonjour 注册失败: {e}"),
        }

        let _ = app.emit(
            "server-status",
            json!({ "running": true, "port": port, "tvConnected": false }),
        );

        if let Err(e) = axum::serve(listener, router).await {
            let _ = app.emit(
                "server-status",
                json!({ "running": false, "port": 0, "tvConnected": false, "error": e.to_string() }),
            );
        }
    });
}

fn register_mdns(port: u16) -> Result<ServiceDaemon, String> {
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let info = ServiceInfo::new(
        "_webcoding._tcp.local.",
        "Web Coding",
        &machine_host(),
        "",
        port,
        None::<std::collections::HashMap<String, String>>,
    )
    .map_err(|e| e.to_string())?;
    daemon.register(info).map_err(|e| e.to_string())?;
    Ok(daemon)
}

fn machine_host() -> String {
    let name = std::process::Command::new("scutil")
        .args(["--get", "LocalHostName"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "webcoding-mac".to_string());
    format!("{}.local.", name.replace(' ', "-"))
}

async fn ws_handler(ws: WebSocketUpgrade, AxumState(app): AxumState<AppHandle>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app))
}

async fn handle_socket(mut socket: WebSocket, app: AppHandle) {
    set_connected(&app, true);

    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(text) => {
                if let Some(event) = parse_event(&text) {
                    let app2 = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let result = run_event(&app2, &event).await;
                        let _ = app2.emit("remote-event", &result);
                    });
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    set_connected(&app, false);
}

fn set_connected(app: &AppHandle, connected: bool) {
    if let Some(state) = app.try_state::<AppState>() {
        state.tv_connected.store(connected, Ordering::Relaxed);
        let port = state.server_port.load(Ordering::Relaxed);
        let _ = app.emit(
            "server-status",
            json!({ "running": port != 0, "port": port, "tvConnected": connected }),
        );
    }
}

fn parse_event(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let event = value.get("event")?.as_str()?.to_string();
    REMOTE_EVENTS
        .contains(&event.as_str())
        .then_some(event)
}

async fn run_event(app: &AppHandle, event: &str) -> serde_json::Value {
    let handle = app.clone();
    let event_owned = event.to_string();
    let event_for_task = event_owned.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let engine = state.engine.lock().unwrap();
        engine.execute(&event_for_task)
    })
    .await;
    match joined {
        Ok(result) => {
            crate::remote::diag::push_result(app, &result);
            serde_json::to_value(&result).unwrap_or_else(|_| json!({}))
        }
        Err(e) => json!({ "event": event_owned, "kind": "none", "value": "", "ok": false, "detail": e.to_string() }),
    }
}
