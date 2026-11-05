import Foundation
import ProxyLogic
struct Preferences: Codable {
    var sources: [Source] = []
    var interval: Int = 60
    var automatic: Bool = true
    var exploreMultihop: Bool? = false
    var routingMode: RoutingMode? = .lowLatency
    var trafficMode: TrafficMode? = .rule
    // 旧版偏好文件没有该字段时按 Mihomo 处理。
    var kernel: KernelKind? = .mihomo
    // 本地混合代理端口（HTTP + SOCKS）；旧版偏好文件缺省时回退默认端口。
    var proxyPort: Int? = Configuration.defaultProxyPort
    // 用户手动固定的出口节点；非空时一键打通直接使用该节点，跳过自动选路。
    var pinnedNodeID: String?
    var chain: ChainOptions? = nil
    var secret: String = UUID().uuidString + UUID().uuidString
    var subscriptionToken: String = UUID().uuidString
}
struct NodeStatus {
    var result: ProbeResult
    var checked: Date
}
enum CoreStartPurpose {
    case connection, latencyTest
    var monitorsConnections: Bool { self == .connection }
}
