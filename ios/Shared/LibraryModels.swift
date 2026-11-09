import Foundation

enum LinkPreference: String, Codable, CaseIterable, Identifiable {
    case lowLatency, highSpeed, ai
    var id: String { rawValue }
    var title: String {
        switch self { case .lowLatency: return "低延迟"; case .highSpeed: return "高速"; case .ai: return "AI" }
    }
    var detail: String { "当前仅保存选路偏好；未运行内核，不会伪造延迟、吞吐或 OpenAI 验证结果。" }
}

struct SubscriptionRecord: Codable, Identifiable {
    var source: Source
    var content: String
    var updatedAt: Date
    var id: UUID { source.id }
}

struct MobileLibrary: Codable {
    var version = 1
    var subscriptions: [SubscriptionRecord] = []
    var selectedNodeID: String?
    var trafficMode: TrafficMode = .rule
    var linkPreference: LinkPreference = .lowLatency
}

enum MobileFailure: LocalizedError {
    case message(String)
    var errorDescription: String? { if case let .message(text) = self { return text }; return nil }
}

enum MobileConfiguration {
    static let maxSubscriptionBytes = 10_000_000
    static let maxLibraryBytes = 24_000_000
    static func parsed(_ library: MobileLibrary) throws -> [ProxyNode] {
        try SubscriptionParser.merge(library.subscriptions.map {
            try SubscriptionParser.parse($0.content, source: $0.source)
        })
    }
    static func export(library: MobileLibrary, node: ProxyNode?) throws -> Data {
        let proxies: [[String: Any]]
        if library.trafficMode == .direct { proxies = node.map { [$0.payload] } ?? [] }
        else {
            guard let node else { throw MobileFailure.message("请先选择节点。") }
            proxies = [node.payload]
        }
        // 这是供用户显式分享的 Clash JSON，不是已运行的 iOS VPN 配置。
        let object: [String: Any] = [
            "mixed-port": 7890, "allow-lan": false, "bind-address": "127.0.0.1",
            "mode": library.trafficMode.rawValue, "log-level": "warning",
            "proxies": proxies,
            "proxy-groups": [["name": "SMART", "type": "select", "proxies": node.map { [$0.name] } ?? ["DIRECT"]]],
            "rules": ["IP-CIDR,10.0.0.0/8,DIRECT,no-resolve", "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve", "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve", "GEOIP,CN,DIRECT", "MATCH,SMART"]
        ]
        return try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
    }
    static func validate(_ library: MobileLibrary) throws {
        guard library.version == 1, library.subscriptions.count <= 32 else {
            throw MobileFailure.message("资料库版本或订阅数量无效（最多 32 份）。")
        }
        guard Set(library.subscriptions.map(\.id)).count == library.subscriptions.count else {
            throw MobileFailure.message("资料库含重复订阅标识。")
        }
        let bytes = library.subscriptions.reduce(0) { $0 + $1.content.utf8.count }
        guard bytes <= maxLibraryBytes else { throw MobileFailure.message("资料库超过 24 MB。") }
        for item in library.subscriptions {
            guard !item.source.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  item.content.utf8.count <= maxSubscriptionBytes else {
                throw MobileFailure.message("订阅名称或内容大小无效。")
            }
        }
        _ = try parsed(library)
    }
}
