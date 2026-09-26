import Foundation
import Combine

@MainActor
final class MappingStore: ObservableObject {
    @Published var mappings: [RemoteEvent: ActionMapping] {
        didSet { save() }
    }

    private let key = "webcoding.remote.mappings.v1"

    init() {
        if let data = UserDefaults.standard.data(forKey: key),
           let decoded = try? JSONDecoder().decode([String: ActionMapping].self, from: data) {
            var restored: [RemoteEvent: ActionMapping] = [:]
            for (raw, mapping) in decoded {
                if let event = RemoteEvent(rawValue: raw) { restored[event] = mapping }
            }
            self.mappings = MappingStore.fillDefaults(restored)
        } else {
            self.mappings = MappingStore.defaultMappings
        }
    }

    private func save() {
        let raw = Dictionary(uniqueKeysWithValues: mappings.map { ($0.key.rawValue, $0.value) })
        if let data = try? JSONEncoder().encode(raw) {
            UserDefaults.standard.set(data, forKey: key)
        }
    }

    static let defaultMappings: [RemoteEvent: ActionMapping] = [
        .up: .init(kind: .arrowUp),
        .down: .init(kind: .arrowDown),
        .left: .init(kind: .arrowLeft),
        .right: .init(kind: .arrowRight),
        .select: .init(kind: .codexApprove),
        .back: .init(kind: .codexReject),
        .playPause: .init(kind: .dictation)
    ]

    private static func fillDefaults(_ input: [RemoteEvent: ActionMapping]) -> [RemoteEvent: ActionMapping] {
        var result = defaultMappings
        for (k, v) in input { result[k] = v }
        return result
    }
}
