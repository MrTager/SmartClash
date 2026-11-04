import Foundation
import CryptoKit

public struct ChainHop: Identifiable, Equatable {
    public let id: String
    public let name: String
    public let sourceID: UUID
    public let protocolName: String
}

public struct ChainTrace: Equatable {
    public let hops: [ChainHop]
    public let complete: Bool
    public static func resolve(selected: String, rawNodes: [ProxyNode], definitions: [ProxyNode]) -> ChainTrace {
        let index = Dictionary((rawNodes + definitions).map { ($0.name, $0) }, uniquingKeysWith: { _, new in new })
        let originals = Dictionary(rawNodes.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
        var cursor: String? = selected
        var visited = Set<String>(); var result: [ChainHop] = []
        while let name = cursor, !name.isEmpty {
            guard visited.insert(name).inserted, result.count < 16, let node = index[name] else {
                return ChainTrace(hops: result.reversed(), complete: false)
            }
            var identity = node.payload; identity.removeValue(forKey: "name"); identity.removeValue(forKey: "dialer-proxy")
            let data = (try? JSONSerialization.data(withJSONObject: identity, options: [.sortedKeys])) ?? Data()
            let fingerprint = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            let original = originals[fingerprint] ?? node
            result.append(ChainHop(id: node.id, name: original.name, sourceID: original.sourceID,
                protocolName: (node.payload["type"] as? String ?? "proxy").uppercased()))
            cursor = node.payload["dialer-proxy"] as? String
        }
        return ChainTrace(hops: result.reversed(), complete: !result.isEmpty)
    }
}
