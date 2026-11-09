import Foundation

public enum RoutingMode: String, Codable, CaseIterable, Identifiable {
    case lowLatency, highSpeed, ai
    public var id: String { rawValue }
    public var title: String {
        switch self { case .lowLatency: return "低延迟链路"; case .highSpeed: return "高速链路"; case .ai: return "AI 链路" }
    }
    public var explanation: String {
        switch self {
        case .lowLatency: return "自动选路优先非中国香港节点，同组按实测 HTTPS 延迟选择；无合适线路时回退。"
        case .highSpeed: return "优先非中国香港节点，按实测吞吐选择；每组最多测 3 个候选、各 1 MiB，含回退最多约 6 MiB。"
        case .ai: return "优先非中国香港节点，线路仍须通过 OpenAI API 可达性检查；不代表 ChatGPT 网页登录或账号权限。"
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
        case .lowLatency: return NodeRanking.preferNonHongKong(ordered)
        case .highSpeed:
            return NodeRanking.preferNonHongKong(ordered.filter { (measurements[$0.id]?.throughput?.bytesPerSecond ?? 0) > 0 }.sorted {
                let left = measurements[$0.id]!.throughput!.bytesPerSecond
                let right = measurements[$1.id]!.throughput!.bytesPerSecond
                return left == right ? $0.id < $1.id : left > right
            })
        case .ai: return NodeRanking.preferNonHongKong(ordered.filter { measurements[$0.id]?.openAIReachable == true })
        }
    }
}
