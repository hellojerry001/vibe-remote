import SwiftUI

@main
struct WebCodingMacApp: App {
    @StateObject private var model = AppModel()

    var body: some Scene {
        MenuBarExtra("Web Coding", systemImage: "appletvremote.gen4") {
            VStack(alignment: .leading, spacing: 10) {
                Text(model.status)
                Text("最近：\(model.lastEvent)").foregroundStyle(.secondary)
                Divider()
                Button("打开设置") { NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil) }
                Button("辅助功能权限") { model.requestAccessibility() }
                Divider()
                Button("退出") { NSApp.terminate(nil) }
            }
            .padding(8)
        }

        Settings {
            ContentView(model: model, store: model.mappings)
        }
    }
}
