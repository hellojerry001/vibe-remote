import Foundation
import Combine

@MainActor
final class AppModel: ObservableObject {
    @Published var status = "正在启动…"
    @Published var lastEvent = "—"

    let mappings = MappingStore()
    private let server = CommandServer()
    private let executor = ActionExecutor()

    init() {
        server.onStatus = { [weak self] value in self?.status = value }
        server.onEvent = { [weak self] event in
            guard let self else { return }
            self.lastEvent = event.displayName
            if let mapping = self.mappings.mappings[event] {
                self.executor.execute(mapping)
            }
        }
        server.start()
    }

    func requestAccessibility() {
        executor.requestAccessibilityPermission()
    }
}
