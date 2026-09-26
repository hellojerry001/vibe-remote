import SwiftUI

struct ContentView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var store: MappingStore

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Web Coding").font(.title2.bold())
                    Text(model.status).foregroundStyle(.secondary)
                }
                Spacer()
                Button("开启辅助功能权限") { model.requestAccessibility() }
            }

            Divider()

            Text("遥控器映射").font(.headline)
            ForEach(RemoteEvent.allCases) { event in
                MappingRow(event: event, store: store)
            }

            Divider()
            HStack {
                Text("最近指令")
                Spacer()
                Text(model.lastEvent).foregroundStyle(.secondary)
            }

            Text("语音输入：建议把 macOS 听写快捷键设为“连按两次 Control 键”，播放/暂停键即可触发。")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        .padding(20)
        .frame(width: 620)
    }
}

private struct MappingRow: View {
    let event: RemoteEvent
    @ObservedObject var store: MappingStore

    var binding: Binding<ActionMapping> {
        Binding(
            get: { store.mappings[event] ?? .init(kind: .none) },
            set: { store.mappings[event] = $0 }
        )
    }

    var body: some View {
        HStack(spacing: 12) {
            Text(event.displayName).frame(width: 90, alignment: .leading)
            Picker("动作", selection: Binding(
                get: { binding.wrappedValue.kind },
                set: { binding.wrappedValue.kind = $0 }
            )) {
                ForEach(ActionKind.allCases) { kind in Text(kind.displayName).tag(kind) }
            }
            .labelsHidden()
            .frame(width: 210)

            if binding.wrappedValue.kind == .keyboardShortcut {
                TextField("例如 cmd+shift+p", text: Binding(
                    get: { binding.wrappedValue.shortcut },
                    set: { binding.wrappedValue.shortcut = $0 }
                ))
                .textFieldStyle(.roundedBorder)
            } else {
                Spacer()
            }
        }
    }
}
