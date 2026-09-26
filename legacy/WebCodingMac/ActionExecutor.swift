import AppKit
import ApplicationServices

final class ActionExecutor {
    func requestAccessibilityPermission() {
        let options = [kAXTrustedCheckOptionPrompt.takeRetainedValue() as String: true] as CFDictionary
        _ = AXIsProcessTrustedWithOptions(options)
    }

    func execute(_ mapping: ActionMapping) {
        switch mapping.kind {
        case .arrowUp: sendKey(code: 126)
        case .arrowDown: sendKey(code: 125)
        case .arrowLeft: sendKey(code: 123)
        case .arrowRight: sendKey(code: 124)
        case .codexApprove:
            if !pressFrontmostButton(containingAny: ["approve", "allow", "confirm", "批准", "允许", "确认"]) {
                sendKey(code: 36) // Return fallback
            }
        case .codexReject:
            if !pressFrontmostButton(containingAny: ["reject", "decline", "deny", "cancel", "拒绝", "取消"]) {
                sendKey(code: 53) // Escape fallback
            }
        case .dictation:
            // 推荐在 macOS「系统设置 → 键盘 → 听写 → 快捷键」中设置为“连按两次 Control 键”。
            doubleTapControl()
        case .keyboardShortcut:
            sendShortcut(mapping.shortcut)
        case .none:
            break
        }
    }

    private func sendKey(code: CGKeyCode, flags: CGEventFlags = []) {
        let source = CGEventSource(stateID: .hidSystemState)
        let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true)
        let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false)
        down?.flags = flags
        up?.flags = flags
        down?.post(tap: .cghidEventTap)
        up?.post(tap: .cghidEventTap)
    }

    private func doubleTapControl() {
        let source = CGEventSource(stateID: .hidSystemState)
        for _ in 0..<2 {
            let down = CGEvent(keyboardEventSource: source, virtualKey: 59, keyDown: true)
            down?.flags = .maskControl
            let up = CGEvent(keyboardEventSource: source, virtualKey: 59, keyDown: false)
            down?.post(tap: .cghidEventTap)
            up?.post(tap: .cghidEventTap)
            usleep(120_000)
        }
    }

    private func sendShortcut(_ spec: String) {
        let parts = spec.lowercased().split(separator: "+").map(String.init)
        guard let keyToken = parts.last, let keyCode = keyCode(for: keyToken) else { return }
        var flags: CGEventFlags = []
        for token in parts.dropLast() {
            switch token {
            case "cmd", "command", "⌘": flags.insert(.maskCommand)
            case "shift", "⇧": flags.insert(.maskShift)
            case "opt", "option", "alt", "⌥": flags.insert(.maskAlternate)
            case "ctrl", "control", "⌃": flags.insert(.maskControl)
            default: break
            }
        }
        sendKey(code: keyCode, flags: flags)
    }

    private func keyCode(for token: String) -> CGKeyCode? {
        let special: [String: CGKeyCode] = [
            "return": 36, "enter": 36, "tab": 48, "space": 49, "escape": 53, "esc": 53,
            "left": 123, "right": 124, "down": 125, "up": 126,
            "delete": 51, "backspace": 51
        ]
        if let code = special[token] { return code }
        let letters: [String: CGKeyCode] = [
            "a":0,"s":1,"d":2,"f":3,"h":4,"g":5,"z":6,"x":7,"c":8,"v":9,
            "b":11,"q":12,"w":13,"e":14,"r":15,"y":16,"t":17,"1":18,"2":19,
            "3":20,"4":21,"6":22,"5":23,"=":24,"9":25,"7":26,"-":27,"8":28,
            "0":29,"]":30,"o":31,"u":32,"[":33,"i":34,"p":35,"l":37,"j":38,
            "'":39,"k":40,";":41,"\\":42,",":43,"/":44,"n":45,"m":46,".":47
        ]
        return letters[token]
    }

    private func pressFrontmostButton(containingAny needles: [String]) -> Bool {
        guard AXIsProcessTrusted(),
              let app = NSWorkspace.shared.frontmostApplication else { return false }

        let axApp = AXUIElementCreateApplication(app.processIdentifier)
        var focused: CFTypeRef?
        let focusedResult = AXUIElementCopyAttributeValue(axApp, kAXFocusedWindowAttribute as CFString, &focused)
        let root: AXUIElement = (focusedResult == .success && focused != nil)
            ? unsafeBitCast(focused, to: AXUIElement.self)
            : axApp

        return findAndPress(in: root, needles: needles.map { $0.lowercased() }, depth: 0)
    }

    private func findAndPress(in element: AXUIElement, needles: [String], depth: Int) -> Bool {
        guard depth < 12 else { return false }

        var roleRef: CFTypeRef?
        var titleRef: CFTypeRef?
        var descRef: CFTypeRef?
        AXUIElementCopyAttributeValue(element, kAXRoleAttribute as CFString, &roleRef)
        AXUIElementCopyAttributeValue(element, kAXTitleAttribute as CFString, &titleRef)
        AXUIElementCopyAttributeValue(element, kAXDescriptionAttribute as CFString, &descRef)

        let role = roleRef as? String ?? ""
        let title = (titleRef as? String ?? "").lowercased()
        let desc = (descRef as? String ?? "").lowercased()
        let haystack = title + " " + desc

        if role == kAXButtonRole as String, needles.contains(where: { haystack.contains($0) }) {
            return AXUIElementPerformAction(element, kAXPressAction as CFString) == .success
        }

        var childrenRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
              let children = childrenRef as? [AXUIElement] else { return false }

        for child in children {
            if findAndPress(in: child, needles: needles, depth: depth + 1) { return true }
        }
        return false
    }
}
