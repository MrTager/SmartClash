import SwiftUI
import ProxyLogic

@MainActor
struct RoutingModeControls: View {
    @EnvironmentObject var store: AppStore
    var body: some View {
        GroupBox("线路模式") {
            VStack(alignment: .leading, spacing: 12) {
                Picker("线路模式", selection: Binding(get: { store.preferences.routingMode ?? .lowLatency }, set: {
                    store.preferences.routingMode = $0; store.savePreferences()
                })) {
                    ForEach(RoutingMode.allCases) { Text($0.title).tag($0) }
                }.pickerStyle(.segmented).disabled(store.busy || store.stopping)
                Text((store.preferences.routingMode ?? .lowLatency).explanation).font(.caption).foregroundStyle(.secondary)
                HStack {
                    Text("当前生效：\(store.activeMode?.title ?? "尚未连接")").font(.caption)
                    Spacer()
                    Button("按此模式重新选路") { store.perform { try await store.applyRoutingMode() } }
                        .disabled(!store.running || store.busy || store.isTestingLatency || store.stopping)
                }
                if let node = store.nodeListNodes.first(where: { $0.name == store.selected }) {
                    HStack(spacing: 20) {
                        if let delay = store.latencies[node.id]?.milliseconds { Text("HTTPS：\(delay) ms") }
                        if let speed = store.measurements[node.id]?.throughput {
                            Text("下载抽测：\(speed.megabitsPerSecond, specifier: "%.2f") Mbps")
                        } else { Text("下载吞吐：未抽测") }
                        if let reachable = store.measurements[node.id]?.openAIReachable {
                            Text(reachable ? "OpenAI：已验证" : "OpenAI：本次未确认")
                        }
                    }.font(.caption).foregroundStyle(.secondary)
                }
            }.padding(8)
        }
    }
}
