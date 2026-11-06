import SwiftUI
import ProxyLogic

@MainActor
struct TrafficModeControls: View {
    @EnvironmentObject var store: AppStore
    var compact = false
    var showsSelection = true

    var body: some View {
        if compact {
            HStack(spacing: 12) {
                Text("代理模式").frame(width: 64, alignment: .leading)
                modePicker
            }
        } else {
            GroupBox("代理模式") {
                VStack(alignment: .leading, spacing: 10) {
                    if showsSelection { modePicker }
                    Text(store.trafficMode.explanation).font(.caption).foregroundStyle(.secondary)
                    Text(effectiveStatus)
                        .font(.caption).foregroundStyle(.secondary)
                    Divider()
                    HStack {
                        Link("规则源：MrTager/SmartClash-Rules", destination: URL(string: RuleSubscriptionService.repository)!)
                        Spacer()
                        Button(store.rulesUpdating ? "正在更新…" : "立即更新规则") {
                            store.perform { try await store.refreshRules() }
                        }.disabled(store.rulesUpdating || store.stopping)
                    }
                    Text("\(store.ruleUpdateMessage) · 应用运行期间每小时自动更新，下载失败保留缓存。")
                        .font(.caption).foregroundStyle(.secondary)
                    if let snapshot = store.ruleSnapshot {
                        Text("最近成功：\(snapshot.updatedAt.formatted(date: .abbreviated, time: .standard)) · 国内域名 / IP 直连规则共 \(snapshot.count) 条")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(8)
            }
        }
    }

    private var effectiveStatus: String {
        if store.stopping { return "正在停止并恢复网络。" }
        if store.recovery { return "网络设置待恢复，暂不确认代理模式已生效。" }
        return store.running ? "当前生效：\(store.trafficMode.title)；切换时会关闭旧连接，应用将重新建立连接。" : "已保存选择，下次连接时生效。"
    }

    private var modePicker: some View {
        Picker("代理模式", selection: Binding(get: { store.trafficMode }, set: { mode in
            store.perform { try await store.changeTrafficMode(mode) }
        })) {
            ForEach(TrafficMode.allCases) { Text($0.title).tag($0) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .accessibilityLabel("代理模式")
        .disabled(store.recovery || store.stopping)
    }
}
