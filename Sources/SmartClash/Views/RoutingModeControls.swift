import SwiftUI
import ProxyLogic

@MainActor
struct RoutingModeControls: View {
    @EnvironmentObject var store: AppStore
    var compact = false
    var showsSelection = true

    var body: some View {
        if compact {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 12) {
                    Text("链路模式").frame(width: 64, alignment: .leading)
                    modePicker
                    rerouteButton.controlSize(.small).fixedSize()
                }
                Text(effectiveStatus).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    .help(effectiveStatus)
            }
        } else {
            GroupBox("链路模式") {
                VStack(alignment: .leading, spacing: 12) {
                    if showsSelection { modePicker }
                    Text((store.preferences.routingMode ?? .lowLatency).explanation).font(.caption).foregroundStyle(.secondary)
                    Text("选择仅保存偏好；点击“重新选路”验证并应用于当前连接，下次连接也会使用该偏好。")
                        .font(.caption).foregroundStyle(.secondary)
                    if showsSelection {
                        HStack {
                            Text(effectiveStatus).font(.caption)
                            Spacer()
                            rerouteButton
                        }
                    }
                    if store.running && !store.stopping && !store.recovery && store.trafficMode != .direct,
                       let node = store.nodeListNodes.first(where: { $0.name == store.selected }) {
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

    private var modePicker: some View {
        Picker("链路模式", selection: Binding(get: { store.preferences.routingMode ?? .lowLatency }, set: {
            store.preferences.routingMode = $0; store.savePreferences()
        })) {
            ForEach(RoutingMode.allCases) { Text($0.title).tag($0) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .accessibilityLabel("链路模式")
    }

    private var rerouteButton: some View {
        Button(compact ? "重新选路" : "按此模式重新选路") {
            store.perform { try await store.applyRoutingMode() }
        }
        .accessibilityLabel("按所选链路模式重新选路")
        .disabled(!store.running || store.trafficMode == .direct || store.busy || store.recovery || store.stopping)
    }

    private var effectiveStatus: String {
        if store.stopping { return "正在停止并恢复网络，选路暂不可用" }
        if store.recovery { return "网络设置待恢复，暂不确认生效链路" }
        guard store.running else { return "当前生效：已停止（链路偏好已保存）" }
        if store.trafficMode == .direct { return "当前生效：直连（代理选路暂停）" }
        guard !store.selected.isEmpty, let activeMode = store.activeMode else {
            return "当前生效：尚未确认链路模式"
        }
        let pending = activeMode != (store.preferences.routingMode ?? .lowLatency)
        return "当前生效：\(activeMode.title)" + (pending ? " · 所选偏好待重新选路" : "")
    }
}
