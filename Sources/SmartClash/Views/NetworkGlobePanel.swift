import SwiftUI
import SceneKit
import ProxyLogic

@MainActor
struct NetworkGlobePanel: View {
    @ObservedObject var activity: NetworkActivityStore
    let nodes: [ProxyNode]
    let trace: ChainTrace
    var compact = false
    @StateObject private var renderer = GlobeScene()
    @State private var focusedConnection: String?
    @State private var showConnections = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var snapshot: GlobeSnapshot {
        GlobeSnapshot.make(nodes: nodes, activeTrace: trace, connections: activity.connections, locations: activity.locations, focusedConnection: focusedConnection)
    }

    var body: some View {
        let data = snapshot
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label("全球连接", systemImage: "globe.asia.australia.fill").font(.headline)
                Spacer()
                Circle().fill(activity.connected ? Color.green : Color.secondary).frame(width: 6, height: 6)
                Text(activity.connected ? "实时 · \(activity.connections.count) 个连接" : (activity.enabled ? "正在连接内核…" : "等待连接"))
                    .font(.caption).foregroundStyle(.secondary)
            }
            ZStack(alignment: .topLeading) {
                GlobeView(renderer: renderer)
                    .accessibilityLabel("3D 地球，\(data.mappedNodes) 个节点位置，\(data.mappedConnections) 个已定位请求。拖动旋转，滚轮缩放。")
                VStack(alignment: .leading, spacing: 6) {
                    Text(trace.hops.isEmpty ? "尚未选择出口" : "当前 \(trace.hops.count) 跳线路")
                        .font(.system(.caption, design: .monospaced)).foregroundStyle(.cyan)
                    if let last = trace.hops.last {
                        Text(last.name.components(separatedBy: " · ").first ?? last.name)
                            .font(.headline).foregroundStyle(.white).lineLimit(1)
                    }
                    if !trace.hops.isEmpty && data.selectedLocation == nil {
                        Text("当前节点位置待解析").font(.caption).foregroundStyle(.white.opacity(0.65))
                    }
                    Text("\(data.mappedNodes) 处节点 · \(data.mappedConnections) 个请求已定位")
                        .font(.caption).foregroundStyle(.white.opacity(0.65))
                    if activity.resolving { Text("正在解析 IP 位置…").font(.caption2).foregroundStyle(.white.opacity(0.5)) }
                    Spacer()
                    HStack(spacing: 16) {
                        legend("代理节点", color: .cyan)
                        legend("请求目标 / 流向", color: .orange)
                        Spacer()
                        Button {
                            if let location = data.selectedLocation { renderer.resetCamera(toward: location) }
                        } label: { Image(systemName: "scope") }
                        .buttonStyle(.borderless).foregroundStyle(.white.opacity(0.85)).help("回到当前节点服务器 IP 的近似位置，并非实际出网 IP")
                        .accessibilityLabel("回到当前节点")
                        .disabled(data.selectedLocation == nil)
                    }
                }.padding(16).allowsHitTesting(true)
                if !activity.enabled {
                    Text("连接后显示节点与请求目标的真实连线")
                        .font(.caption).foregroundStyle(.white.opacity(0.65))
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom).padding(.bottom, 49)
                        .allowsHitTesting(false)
                }
            }
            .frame(height: compact ? 270 : 330)
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(.primary.opacity(0.08)))
            .onAppear { renderer.update(data, animate: !reduceMotion) }
            .onChange(of: data) { renderer.update($0, animate: !reduceMotion) }
            .onChange(of: reduceMotion) { renderer.update(data, animate: !$0) }
            HStack {
                Text("拖动旋转 · 滚轮缩放 · IP 位置为近似值")
                Spacer()
                Button(showConnections ? "收起请求" : "查看实时请求") { showConnections.toggle() }
                    .buttonStyle(.link)
            }.font(.caption).foregroundStyle(.secondary)
            if showConnections {
                connections
            }
            Text("节点位置为节点服务器 IP 的近似位置，并非实际出网 IP。IP 位置由 ipwho.is 提供并缓存 24 小时；目标 IP 缺失时按本机 DNS 估算。未定位与直连请求保留在列表。连线表示地理关系。")
                .font(.caption2).foregroundStyle(.secondary)
        }
        .onChange(of: trace) { _ in focusedConnection = nil }
        .onChange(of: activity.connections.map(\.id)) { ids in
            if let focusedConnection, !ids.contains(focusedConnection) { self.focusedConnection = nil }
        }
    }

    private func legend(_ title: String, color: Color) -> some View {
        HStack(spacing: 5) {
            Circle().fill(color).frame(width: 6, height: 6)
            Text(title).font(.caption2).foregroundStyle(.white.opacity(0.75))
        }
    }

    private var connections: some View {
        VStack(alignment: .leading, spacing: 0) {
            if activity.connections.isEmpty {
                Label(activity.enabled ? "当前没有活跃请求" : "连接后显示实时请求", systemImage: "network")
                    .font(.caption).foregroundStyle(.secondary).padding(.vertical, 14)
            } else {
                HStack {
                    Text("请求目标与实际节点").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    if focusedConnection != nil { Button("显示全部连线") { focusedConnection = nil }.buttonStyle(.link).font(.caption) }
                }.padding(.bottom, 8)
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(activity.connections.prefix(80)) { connection in
                            Button {
                                focusedConnection = focusedConnection == connection.id ? nil : connection.id
                                if let server = connection.routeNodes(in: nodes).last?.payload["server"] as? String,
                                   let location = activity.locations[server] { renderer.resetCamera(toward: location) }
                            } label: { connectionRow(connection) }
                            .buttonStyle(.plain)
                            Divider()
                        }
                    }
                }.frame(height: min(CGFloat(activity.connections.count) * 65, 230))
                if activity.connections.count > 80 { Text("显示流量最高的 80 个连接").font(.caption2).foregroundStyle(.secondary) }
            }
        }
    }

    private func connectionRow(_ connection: CoreConnection) -> some View {
        let location = connection.metadata.geolocationKey.flatMap { activity.locations[$0] }
        return HStack(spacing: 10) {
            Circle().fill(location == nil ? Color.secondary : .orange).frame(width: 6, height: 6)
            VStack(alignment: .leading, spacing: 4) {
                Text(connection.metadata.displayDestination).font(.system(.caption, design: .monospaced)).lineLimit(1)
                Text(connection.pathLabel.isEmpty ? "节点路径未知" : connection.pathLabel)
                    .font(.caption2).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 4) {
                Text(location.map { $0.label + (connection.metadata.publicDestinationIP == nil ? " · DNS 估算" : "") } ?? "位置未解析")
                    .font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                Text("↑ \(TrafficStore.bytes(connection.upload))  ↓ \(TrafficStore.bytes(connection.download))")
                    .font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
            }
        }
        .padding(8).frame(maxWidth: .infinity, alignment: .leading)
        .background(focusedConnection == connection.id ? Color.accentColor.opacity(0.1) : .clear, in: RoundedRectangle(cornerRadius: 6))
        .contentShape(Rectangle())
        .help("点击聚焦此请求的节点连线")
    }
}
