import SwiftUI
import UIKit

struct RemoteView: View {
    @ObservedObject var connector: MacConnector

    var body: some View {
        ZStack {
            // 全屏长按捕获层：按住触摸表面 0.6s 触发 centerLongPress
            LongPressCatcher { connector.send(.centerLongPress) }

            VStack(spacing: 34) {
                Text("Web Coding")
                    .font(.system(size: 56, weight: .bold, design: .rounded))

                Text(connector.status)
                    .font(.title3)
                    .foregroundStyle(.secondary)

                VStack(spacing: 14) {
                    Text("⌃  滑动：移动焦点")
                    Text("●  确认：执行当前映射")
                    Text("●  确认长按 0.6s：长按动作")
                    Text("‹  返回：返回键")
                    Text("▶︎❚❚  播放/暂停：语音输入")
                }
                .font(.title3)
                .multilineTextAlignment(.center)

                Button("测试") { connector.send(.center) }
                    .buttonStyle(.borderedProminent)
            }
            .padding(60)
        }
        .onMoveCommand { direction in
            switch direction {
            case .up: connector.send(.up)
            case .down: connector.send(.down)
            case .left: connector.send(.left)
            case .right: connector.send(.right)
            @unknown default: break
            }
        }
        .onExitCommand { connector.send(.back) }
        .onPlayPauseCommand { connector.send(.playPause) }
    }
}

/// UIViewRepresentable 长按手势层，tvOS 上比 SwiftUI LongPressGesture 与焦点系统的兼容性更好
private struct LongPressCatcher: UIViewRepresentable {
    let onLongPress: () -> Void

    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.backgroundColor = .clear
        let gesture = UILongPressGestureRecognizer(
            target: context.coordinator,
            action: #selector(Coordinator.handle(_:))
        )
        gesture.minimumPressDuration = 0.6
        gesture.cancelsTouchesInView = true
        view.addGestureRecognizer(gesture)
        return view
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        context.coordinator.onLongPress = onLongPress
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(onLongPress: onLongPress)
    }

    final class Coordinator: NSObject {
        var onLongPress: () -> Void

        init(onLongPress: @escaping () -> Void) {
            self.onLongPress = onLongPress
        }

        @objc func handle(_ gesture: UILongPressGestureRecognizer) {
            if gesture.state == .began {
                onLongPress()
            }
        }
    }
}
