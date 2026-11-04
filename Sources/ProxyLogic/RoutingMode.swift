import Foundation

public enum RoutingMode: String, Codable, CaseIterable, Identifiable {
    case lowLatency, highSpeed, ai
    public var id: String { rawValue }
    public var title: String {
        switch self { case .lowLatency: return "低延迟链路"; case .highSpeed: return "高速链路"; case .ai: return "AI 链路" }
    }
    public var explanation: String {
        switch self {
        case .lowLatency: return "优先选择实测 HTTPS 延迟较低、出网正常的线路。"
        case .highSpeed: return "对最多 3 个候选各下载 1 MiB，按实测吞吐选择；每轮约消耗 3 MiB 测速流量。"
        case .ai: return "线路必须通过 OpenAI API 可达性检查；不代表 ChatGPT 网页登录或账号权限。"
        }
    }
}

public struct ThroughputSample: Codable, Equatable, Sendable {
    public let bytesPerSecond: Double
    public let downloadedBytes: Int
    public let checked: Date
    public init(bytesPerSecond: Double, downloadedBytes: Int, checked: Date = Date()) {
        self.bytesPerSecond = bytesPerSecond; self.downloadedBytes = downloadedBytes; self.checked = checked
    }
    public var megabitsPerSecond: Double { bytesPerSecond * 8 / 1_000_000 }
}

public struct LinkMeasurements: Codable, Equatable {
    public var throughput: ThroughputSample?
    public var openAIReachable: Bool?
    public var openAIChecked: Date?
    public init(throughput: ThroughputSample? = nil, openAIReachable: Bool? = nil, openAIChecked: Date? = nil) {
        self.throughput = throughput; self.openAIReachable = openAIReachable; self.openAIChecked = openAIChecked
    }
}

public enum ModeRanking {
    public static func best(_ nodes: [ProxyNode], mode: RoutingMode, latencies: [String: NodeLatency], measurements: [String: LinkMeasurements]) -> [ProxyNode] {
        let ordered = NodeRanking.sorted(nodes, latencies: latencies)
        switch mode {
        case .lowLatency: return ordered
        case .highSpeed:
            return ordered.filter { (measurements[$0.id]?.throughput?.bytesPerSecond ?? 0) > 0 }.sorted {
                let left = measurements[$0.id]!.throughput!.bytesPerSecond
                let right = measurements[$1.id]!.throughput!.bytesPerSecond
                return left == right ? $0.id < $1.id : left > right
            }
        case .ai: return ordered.filter { measurements[$0.id]?.openAIReachable == true }
        }
    }
}
