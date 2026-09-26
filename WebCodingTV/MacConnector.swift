import Foundation
import Network
import Combine

/// tvOS → Mac 连接层：
/// 1. NWBrowser 通过 Bonjour 发现 `_webcoding._tcp` 服务；
/// 2. 用一条临时 TCP 连接解析出真实 host/port；
/// 3. 升级为 WebSocket（`ws://host:port/ws`，与 Mac 端 axum 服务对接）。
@MainActor
final class MacConnector: ObservableObject {
    @Published var status = "正在查找 Mac…"
    @Published var isConnected = false

    private var browser: NWBrowser?
    private var probe: NWConnection?
    private var webSocket: URLSessionWebSocketTask?
    private let queue = DispatchQueue(label: "webcoding.tv.network")

    func start() {
        let browser = NWBrowser(for: .bonjour(type: "_webcoding._tcp", domain: nil), using: .tcp)
        browser.stateUpdateHandler = { [weak self] state in
            DispatchQueue.main.async {
                if case .failed(let error) = state {
                    self?.status = "发现失败：\(error.localizedDescription)"
                }
            }
        }
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            guard let endpoint = results.first?.endpoint else { return }
            self?.resolveAndConnect(endpoint)
        }
        browser.start(queue: queue)
        self.browser = browser
    }

    private func resolveAndConnect(_ endpoint: NWEndpoint) {
        guard probe == nil, webSocket == nil else { return }
        let connection = NWConnection(to: endpoint, using: .tcp)
        connection.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready:
                if let remote = connection.currentPath?.remoteEndpoint,
                   case let .hostPort(host, port) = remote {
                    self?.openWebSocket(host: Self.clean(host), port: port.rawValue)
                }
                self?.probe = nil
                connection.cancel()
            case .failed(let error):
                DispatchQueue.main.async {
                    self?.status = "连接 Mac 失败：\(error.localizedDescription)"
                }
                self?.probe = nil
            default:
                break
            }
        }
        connection.start(queue: queue)
        self.probe = connection
    }

    private func openWebSocket(host: String, port: UInt16) {
        guard let url = URL(string: "ws://\(host):\(port)/ws") else {
            status = "无效的 Mac 地址：\(host):\(port)"
            return
        }
        let task = URLSession(configuration: .default).webSocketTask(with: url)
        task.resume()
        self.webSocket = task
        receiveLoop(task)
        status = "已连接 Mac（\(host):\(port)）"
        isConnected = true
    }

    private func receiveLoop(_ task: URLSessionWebSocketTask) {
        task.receive { [weak self] result in
            guard let self else { return }
            switch result {
            case .success:
                self.receiveLoop(task)
            case .failure:
                DispatchQueue.main.async {
                    self.webSocket = nil
                    self.isConnected = false
                    self.status = "连接断开，正在重新查找 Mac…"
                    self.start()
                }
            }
        }
    }

    func send(_ event: RemoteEvent) {
        guard let webSocket else { return }
        let packet = RemotePacket(event: event, sentAt: Date())
        guard let data = try? JSONEncoder().encode(packet),
              let text = String(data: data, encoding: .utf8) else { return }
        webSocket.send(.string(text)) { [weak self] error in
            if let error {
                DispatchQueue.main.async {
                    self?.status = "发送失败：\(error.localizedDescription)"
                }
            }
        }
    }

    /// 去掉 IPv6 zone（如 `fe80::1%en0`）
    private static func clean(_ host: NWEndpoint.Host) -> String {
        let text = "\(host)"
        return text.split(separator: "%").first.map(String.init) ?? text
    }
}
