import SwiftUI
import ProxyLogic
@MainActor
struct ChainView: View {
    @EnvironmentObject var store: AppStore
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                HStack {
                    VStack(alignment: .leading, spacing: 7) {
                        Text("链路运行情况").font(.largeTitle.bold())
                        Label(store.routingStatus, systemImage: store.running ? "network" : "network.slash")
                            .foregroundStyle(store.running ? (store.routingStatus == "已打通" ? Color.green : Color.orange) : Color.secondary)
                    }
                    Spacer()
                    if store.running || store.busy || store.recovery {
                        Button(store.stopping ? "正在停止…" : "停止并恢复网络") { store.requestStop() }
                            .buttonStyle(.borderedProminent).disabled(store.stopping)
                    } else {
                        Button("连接所选模式") { store.perform { try await store.oneClickConnect() } }
                            .buttonStyle(.borderedProminent).disabled(store.isTestingLatency)
                    }
                }
                if store.busy { HStack { ProgressView().controlSize(.small); Text("正在执行操作，可随时停止").font(.caption) } }
                RoutingModeControls().environmentObject(store)
                GroupBox("当前实际生效路径") {
                    VStack(alignment: .leading, spacing: 14) {
                        if store.activeTrace.hops.isEmpty {
                            Label("尚未连接；连接成功后显示真实节点顺序", systemImage: "point.3.connected.trianglepath.dotted")
                                .foregroundStyle(.secondary).padding(16)
                        } else {
                            ScrollView(.horizontal) {
                                HStack(spacing: 10) {
                                    hop("本机", detail: store.tun ? "TUN" : "HTTP / SOCKS", icon: "laptopcomputer")
                                    ForEach(Array(store.activeTrace.hops.enumerated()), id: \.element.id) { index, node in
                                        Image(systemName: "arrow.right").foregroundStyle(.secondary)
                                        hop(node.name.components(separatedBy: " · ").first ?? node.name,
                                            detail: "第 \(index + 1) 跳 · \(node.protocolName)", icon: "server.rack")
                                    }
                                    Image(systemName: "arrow.right").foregroundStyle(.secondary)
                                    hop(store.activeMode == .ai ? "OpenAI API" : "互联网", detail: "出网目标", icon: "globe")
                                }.padding(10)
                            }
                            Text("\(store.activeTrace.hops.count) 跳 · \(store.standbyReadyCount) 条近期验证通过的备用线路")
                                .font(.caption).foregroundStyle(.secondary)
                            if !store.activeTrace.complete {
                                Label("路径引用不完整，不能确认全部中间节点", systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                            }
                        }
                        Text(store.internetStatus).font(.caption)
                        Text(store.openAIStatus).font(.caption).foregroundStyle(.secondary)
                    }.padding(8)
                }
                TrafficPanel(traffic: store.traffic)
                Toggle("探索多跳链路（高级）", isOn: Binding(get: { store.preferences.exploreMultihop == true }, set: {
                    store.preferences.exploreMultihop = $0; store.savePreferences()
                })).toggleStyle(.checkbox).disabled(store.running || store.busy || store.isTestingLatency)
                Text("图中路径来自当前选中节点及实际 dialer-proxy 引用，不以动画假装连接。后台守护不会锁住页面；已有 TCP 连接在出口切换时仍可能需要重连。")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }
    private func hop(_ title: String, detail: String, icon: String) -> some View {
        VStack(spacing: 9) {
            Image(systemName: icon).font(.title2).foregroundStyle(.blue)
            Text(title).font(.headline).lineLimit(2).multilineTextAlignment(.center).frame(width: 145)
            Text(detail).font(.caption).foregroundStyle(.secondary)
        }.padding(14).background(.quaternary, in: RoundedRectangle(cornerRadius: 12))
    }
}
