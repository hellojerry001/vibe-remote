import SwiftUI

@main
struct WebCodingTVApp: App {
    @StateObject private var connector = MacConnector()

    var body: some Scene {
        WindowGroup {
            RemoteView(connector: connector)
                .onAppear { connector.start() }
        }
    }
}
