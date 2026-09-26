import Foundation

enum RemoteEvent: String, Codable, CaseIterable, Identifiable {
    case up, down, left, right, center, back, playPause, centerLongPress
    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .up: return "上"
        case .down: return "下"
        case .left: return "左"
        case .right: return "右"
        case .center: return "确认键"
        case .back: return "返回键"
        case .playPause: return "播放/暂停"
        case .centerLongPress: return "确认长按"
        }
    }
}

struct RemotePacket: Codable {
    let event: RemoteEvent
    let sentAt: Date
}

enum ActionKind: String, Codable, CaseIterable, Identifiable {
    case arrowUp
    case arrowDown
    case arrowLeft
    case arrowRight
    case codexApprove
    case codexReject
    case dictation
    case keyboardShortcut
    case none

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .arrowUp: return "方向键 ↑"
        case .arrowDown: return "方向键 ↓"
        case .arrowLeft: return "方向键 ←"
        case .arrowRight: return "方向键 →"
        case .codexApprove: return "Codex：批准"
        case .codexReject: return "Codex：拒绝"
        case .dictation: return "Mac 语音输入"
        case .keyboardShortcut: return "自定义快捷键"
        case .none: return "不执行"
        }
    }
}

struct ActionMapping: Codable, Equatable {
    var kind: ActionKind
    var shortcut: String = ""
}
