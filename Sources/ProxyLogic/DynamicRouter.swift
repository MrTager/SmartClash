import Foundation
import CryptoKit
public struct RoutePlan {
    public var candidates: [ProxyNode]
    public var definitions: [ProxyNode]
    public init(candidates: [ProxyNode], definitions: [ProxyNode]) { self.candidates = candidates; self.definitions = definitions }
}
public enum DynamicRouter {
    // Search budget bounds work, not a fixed two/three-hop topology. Every round explores new paths.
    public static func plan(nodes: [ProxyNode], round: Int, budget: Int = 12, allowChains: Bool = true) throws -> RoutePlan {
        guard !nodes.isEmpty, budget > 0 else { throw ProxyError.message("没有可探索的节点") }
        if !allowChains {
            let count = min(budget, nodes.count)
            let offset = ((round % nodes.count) * count) % nodes.count
            return RoutePlan(candidates: (0..<count).map { nodes[(offset + $0) % nodes.count] }, definitions: [])
        }
        let tcp: Set<String> = ["ss", "vmess", "vless", "trojan", "socks5", "http", "snell", "anytls"]
        let pool = nodes.filter { tcp.contains($0.payload["type"] as? String ?? "") }
        var candidates: [ProxyNode] = []
        var definitions: [String: ProxyNode] = [:]
        var seen = Set<String>()
        for slot in 0..<budget {
            // Alternate direct exits and increasingly deep chains; rotate both order and depth across rounds.
            let depth = pool.count < 2 ? 1 : 1 + (round + slot) % min(pool.count, budget)
            if depth == 1 {
                let node = nodes[(round * budget + slot) % nodes.count]
                if seen.insert(node.id).inserted { candidates.append(node) }
                continue
            }
            let start = (round * budget + slot) % pool.count
            let reverse = (round / max(1, pool.count)) % 2 == 1
            var previous = ""
            var pathIDs: [String] = []
            var last: ProxyNode?
            for index in 0..<depth {
                let offset = reverse ? (pool.count - index) % pool.count : index
                let node = pool[(start + offset) % pool.count]
                pathIDs.append(node.id)
                if index == 0 { previous = node.name; continue }
                let digest = SHA256.hash(data: Data(pathIDs.joined(separator: "/").utf8)).map { String(format: "%02x", $0) }.joined()
                let name = "AUTO-\(digest.prefix(24))"
                var payload = node.payload; payload["name"] = name; payload["dialer-proxy"] = previous
                let item = ProxyNode(id: "auto-" + digest, name: name, sourceID: node.sourceID, payload: payload, exitName: node.exitName)
                definitions[item.id] = item; previous = name; last = item
            }
            if let last, seen.insert(last.id).inserted { candidates.append(last) }
        }
        return RoutePlan(candidates: candidates, definitions: definitions.values.sorted { $0.id < $1.id })
    }
}
