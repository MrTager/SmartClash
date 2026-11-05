import SwiftUI
import ProxyLogic

@MainActor
struct TrafficModeControls: View {
    @EnvironmentObject var store: AppStore
    var body: some View {
        GroupBox("流量模式") {
            VStack(alignment: .leading, spacing: 10) {
                Picker("流量模式", selection: Binding(get: { store.trafficMode }, set: { mode in
                    store.perform { try await store.changeTrafficMode(mode) }
                })) {
                    ForEach(TrafficMode.allCases) { Text($0.title).tag($0) }
                }.pickerStyle(.segmented).disabled(store.recovery || store.stopping)
                Text(store.trafficMode.explanation).font(.caption).foregroundStyle(.secondary)
                Text(store.running ? "当前生效：\(store.trafficMode.title)；切换时会关闭旧连接，应用将重新建立连接。" : "已保存选择，下次连接时生效。")
                    .font(.caption).foregroundStyle(.secondary)
                Divider()
                HStack {
                    Link("规则源：MrTager/SmartClash-Rules", destination: URL(string: RuleSubscriptionService.repository)!)
                    Spacer()
                    Button(store.rulesUpdating ? "正在更新…" : "立即更新规则") {
                        store.perform { try await store.refreshRules() }
                    }.disabled(store.rulesUpdating)
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
