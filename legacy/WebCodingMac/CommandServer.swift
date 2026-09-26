import Foundation
import Network

final class CommandServer {
    var onEvent: ((RemoteEvent) -> Void)?
    var onStatus: ((String) -> Void)?

    private var listener: NWListener?
    private let queue = DispatchQueue(label: "webcoding.server")

    func start() {
        do {
            let listener = try NWListener(using: .tcp)
            listener.service = NWListener.Service(name: Host.current().localizedName ?? "Mac", type: "_webcoding._tcp")
            listener.stateUpdateHandler = { [weak self] state in
                DispatchQueue.main.async {
                    switch state {
                    case .ready: self?.onStatus?("等待 Apple TV 连接")
                    case .failed(let error): self?.onStatus?("服务错误：\(error.localizedDescription)")
                    default: break
                    }
                }
            }
            listener.newConnectionHandler = { [weak self] connection in
                self?.accept(connection)
            }
            listener.start(queue: queue)
            self.listener = listener
        } catch {
            onStatus?("无法启动服务：\(error.localizedDescription)")
        }
    }

    private func accept(_ connection: NWConnection) {
        connection.stateUpdateHandler = { [weak self] state in
            DispatchQueue.main.async {
                switch state {
                case .ready: self?.onStatus?("Apple TV 已连接")
                case .failed, .cancelled: self?.onStatus?("等待 Apple TV 连接")
                default: break
                }
            }
        }
        connection.start(queue: queue)
        receiveLoop(connection, buffer: Data())
    }

    private func receiveLoop(_ connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 4096) { [weak self] content, _, isComplete, error in
            guard let self else { return }
            var working = buffer
            if let content { working.append(content) }

            while let newline = working.firstIndex(of: 0x0A) {
                let frame = working.prefix(upTo: newline)
                working.removeSubrange(...newline)
                if let packet = try? JSONDecoder().decode(RemotePacket.self, from: Data(frame)) {
                    DispatchQueue.main.async { self.onEvent?(packet.event) }
                }
            }

            if error == nil && !isComplete {
                self.receiveLoop(connection, buffer: working)
            }
        }
    }
}
