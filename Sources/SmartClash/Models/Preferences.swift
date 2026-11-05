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
