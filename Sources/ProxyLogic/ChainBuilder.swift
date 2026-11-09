import Foundation
public struct ChainOptions: Codable, Equatable {
    public var enabled = false
    public var entryIDs: Set<String> = []
    public var exitIDs: Set<String> = []
    public init() {}
}
public enum ChainBuilder {
    public static func routes(nodes: [ProxyNode], options: ChainOptions) throws -> [ProxyNode] {
        guard options.enabled else { return nodes }
        let entries = nodes.filter { options.entryIDs.contains($0.id) }
        let exits = nodes.filter { options.exitIDs.contains($0.id) }
        guard !entries.isEmpty, !exits.isEmpty else { throw ProxyError.message("链式模式需要至少一个入口和一个出口节点，请重新选择已失效的节点") }
        guard entries.count * exits.count <= 128 else { throw ProxyError.message("入口 × 出口最多 128 个组合，请减少候选节点") }
        // UDP-specific protocols need end-to-end UDP support. Start with explicit TCP-compatible chains.
        let tcp: Set<String> = ["ss", "vmess", "vless", "trojan", "socks5", "http", "snell", "anytls"]
        guard (entries + exits).allSatisfy({ tcp.contains($0.payload["type"] as? String ?? "") }) else {
            throw ProxyError.message("链式模式暂支持 SS、VMess、VLESS、Trojan、SOCKS5、HTTP、Snell、AnyTLS；请勿选择 UDP 专用或 SSR 节点")
        }
        var result: [ProxyNode] = []
        for entry in entries {
            for exit in exits where exit.id != entry.id {
                let id = "chain-\(entry.id)-\(exit.id)"
                let name = "链路 · \(entry.name) → \(exit.name)"
                var payload = exit.payload
                payload["name"] = name
                // Exit connects THROUGH the raw entry. Never reference a generated chain or group.
                payload["dialer-proxy"] = entry.name
                result.append(ProxyNode(id: id, name: name, sourceID: exit.sourceID, payload: payload, exitName: exit.exitName))
            }
        }
        guard !result.isEmpty else { throw ProxyError.message("入口和出口不能是同一个节点") }
        return result
    }
}
