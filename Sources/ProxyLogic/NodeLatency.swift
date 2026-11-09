import Foundation

public struct NodeLatency: Codable, Equatable, Sendable {
    public let milliseconds: Int?
    public let checked: Date
    public let failure: String?
    public init(milliseconds: Int?, checked: Date = Date(), failure: String? = nil) {
        self.milliseconds = milliseconds
        self.checked = checked
        self.failure = failure
    }
    public var reachable: Bool { milliseconds.map { $0 >= 0 } ?? false }
    public func isFresh(now: Date = Date(), lifetime: TimeInterval = 300) -> Bool {
        let age = now.timeIntervalSince(checked)
        return age >= 0 && age <= lifetime
    }
}

public enum NodeSortOrder: String, CaseIterable, Identifiable {
    case latency = "延迟从低到高", name = "节点名称", original = "原始顺序"
    public var id: String { rawValue }
}

public enum NodeRanking {
    // Subscription labels are hints, not proof of exit IP location or OpenAI support.
    public static func isHongKong(_ node: ProxyNode) -> Bool {
        let label = node.exitName.lowercased()
        if label.contains("香港") || label.contains("\u{1f1ed}\u{1f1f0}") { return true }
        let words = label.components(separatedBy: CharacterSet.letters.inverted).filter { !$0.isEmpty }
        if words.contains("hk") || words.contains("hkg") || words.contains("hongkong") { return true }
        return zip(words, words.dropFirst()).contains { $0.0 == "hong" && $0.1 == "kong" }
    }

    // Stable partition: keep metric/source order within each tier and retain fallback exits.
    public static func preferNonHongKong(_ nodes: [ProxyNode]) -> [ProxyNode] {
        nodes.filter { !isHongKong($0) } + nodes.filter { isHongKong($0) }
    }

    public static func sorted(_ nodes: [ProxyNode], latencies: [String: NodeLatency], order: NodeSortOrder = .latency) -> [ProxyNode] {
        guard order != .original else { return nodes }
        return nodes.sorted { a, b in
            if order == .latency {
                let left = latencies[a.id], right = latencies[b.id]
                let l = left?.reachable == true ? 0 : (left == nil ? 1 : 2)
                let r = right?.reachable == true ? 0 : (right == nil ? 1 : 2)
                if l != r { return l < r }
                if l == 0, left?.milliseconds != right?.milliseconds {
                    return left!.milliseconds! < right!.milliseconds!
                }
            }
            let comparison = a.name.localizedStandardCompare(b.name)
            return comparison == .orderedSame ? a.id < b.id : comparison == .orderedAscending
        }
    }

    // Fresh successes first, then round-robin across sources instead of exhausting one source.
    public static func connectionCandidates(_ nodes: [ProxyNode], latencies: [String: NodeLatency], now: Date = Date()) -> [ProxyNode] {
        let known = sorted(nodes.filter { latencies[$0.id]?.reachable == true && latencies[$0.id]?.isFresh(now: now) == true }, latencies: latencies)
        var seen = Set(known.map(\.id))
        var sourceOrder: [UUID] = []
        var pools: [UUID: [ProxyNode]] = [:]
        for node in nodes where !seen.contains(node.id) {
            if pools[node.sourceID] == nil { sourceOrder.append(node.sourceID) }
            pools[node.sourceID, default: []].append(node)
        }
        var result = known
        let rounds = pools.values.map(\.count).max() ?? 0
        for index in 0..<rounds {
            for source in sourceOrder {
                if let pool = pools[source], index < pool.count, seen.insert(pool[index].id).inserted { result.append(pool[index]) }
            }
        }
        return preferNonHongKong(result)
    }
}

public enum DelayEndpoint {
    public static let target = "https://cp.cloudflare.com/generate_204"
    public static func path(name: String, timeout: Int = 5000) -> String {
        let segment = name.addingPercentEncoding(withAllowedCharacters: .alphanumerics)!
        var query = URLComponents()
        query.queryItems = [URLQueryItem(name: "url", value: target), URLQueryItem(name: "timeout", value: String(timeout)), URLQueryItem(name: "expected", value: "204")]
        return "/proxies/\(segment)/delay?\(query.percentEncodedQuery!)"
    }
}

public enum InternetPolicy {
    public static func classify(code: Int, elapsed: Int, transportError: String? = nil) -> ProbeResult {
        let healthy = code == 204
        let detail: String
        if healthy { detail = "HTTPS 出网正常" }
        else if let transportError, !transportError.isEmpty {
            detail = "HTTPS 出网未通过（HTTP \(code)，\(transportError)）"
        } else { detail = "HTTPS 出网未通过（HTTP \(code)）" }
        return ProbeResult(health: healthy ? .healthy : .offline, milliseconds: elapsed, detail: detail)
    }
}
