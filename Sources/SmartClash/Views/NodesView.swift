import SwiftUI
import ProxyLogic

@MainActor
struct NodesView: View {
    @EnvironmentObject var store: AppStore
    @State private var query = ""
    @State private var sort: NodeSortOrder = .latency
    private var visible: [ProxyNode] {
        NodeRanking.sorted(store.nodeListNodes.filter {
            query.isEmpty || $0.name.localizedCaseInsensitiveContains(query)
        }, latencies: store.latencies, order: sort)
    }
    var body: some View {
        ScrollView {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("节点与延迟").font(.largeTitle.bold())
                Spacer()
                TextField("搜索节点", text: $query).frame(width: 200)
            }
            TrafficRateBar(traffic: store.traffic)
            NetworkGlobePanel(activity: store.activity, nodes: store.visualizationNodes, trace: store.activeTrace, compact: true)
            HStack {
                Picker("排序", selection: $sort) {
                    ForEach(NodeSortOrder.allCases) { Text($0.rawValue).tag($0) }
                }.frame(width: 230)
                Text("\(visible.count) 个节点").foregroundStyle(.secondary)
                Spacer()
                if store.isTestingLatency {
                    Text("后台测速 · \(store.latencyCompleted)/\(store.latencyTotal)").monospacedDigit().foregroundStyle(.secondary)
                    Button("取消测速") { store.cancelLatencyTest() }
                }
                Button("全部测速") { store.beginLatencyTest() }
                    .buttonStyle(.borderedProminent)
                    .disabled(store.recovery || store.nodes.isEmpty)
            }
            Text("测速在后台进行，保留已有结果，完成后自动更新。6 路并发、单次最多 5 秒；不会切换当前业务出口或开启系统代理 / TUN。")
                .font(.caption).foregroundStyle(.secondary)
            LazyVStack(spacing: 0) {
            ForEach(visible) { node in
                HStack(spacing: 12) {
                    Image(systemName: node.name == store.selected ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(node.name == store.selected ? .green : .secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 6) {
                            if store.preferences.pinnedNodeID == node.id {
                                Image(systemName: "pin.fill").foregroundStyle(.orange)
                                    .accessibilityLabel("已固定出口")
                            }
                            Text(node.name).lineLimit(1)
                        }
                        HStack {
                            Text(store.preferences.sources.first(where: { $0.id == node.sourceID })?.name ?? "动态链路")
                            if let sample = store.latencies[node.id] {
                                Text(sample.checked.formatted(date: .omitted, time: .standard))
                            }
                        }.font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer()
                    latency(node)
                        .frame(minWidth: 105, alignment: .trailing)
                    Button(store.preferences.pinnedNodeID == node.id ? "取消固定" : "固定") {
                        store.pin(store.preferences.pinnedNodeID == node.id ? nil : node)
                    }
                    .disabled(store.recovery)
                    Button("测速") { store.beginLatencyTest([node]) }
                        .disabled(store.recovery)
                    Button("选择") { store.perform { try await store.selectVerified(node) } }
                        .disabled(!store.running)
                }.padding(.vertical, 7)
                Divider()
            }
            }
            Text("可达节点按延迟从低到高排序，未测与失败节点靠后。延迟不是下载速度，也不代表 OpenAI / ChatGPT 的地区资格。")
                .font(.caption).foregroundStyle(.secondary)
            Text("点「固定」锁定出口：一键打通会直接连该节点并验证出网，跳过自动选路，同时关闭自动切换；「选择」只在当前运行中切换。")
                .font(.caption).foregroundStyle(.secondary)
        }
        }
    }
    @ViewBuilder
    private func latency(_ node: ProxyNode) -> some View {
        if let sample = store.latencies[node.id] {
            if let ms = sample.milliseconds, sample.reachable {
                Text("\(ms) ms").monospacedDigit().foregroundStyle(ms < 1000 ? Color.green : Color.orange)
                    .accessibilityLabel("HTTPS 延迟 \(ms) 毫秒")
            } else {
                Text(sample.failure ?? "不可达").foregroundStyle(.red).font(.caption)
            }
        } else { Text("未测速").foregroundStyle(.secondary).font(.caption) }
    }
}
