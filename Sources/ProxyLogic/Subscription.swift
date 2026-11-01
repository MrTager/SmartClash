import Foundation
import CryptoKit
import Yams

public struct Source: Codable, Identifiable, Equatable {
    public var id: UUID
    public var name: String
    public var url: String
    public init(name: String, url: String) { id = UUID(); self.name = name; self.url = url }
}
public struct ProxyNode: Identifiable {
    public let id: String
    public let name: String
    public let sourceID: UUID
    public let payload: [String: Any]
}
public enum ProxyError: LocalizedError {
    case message(String)
    public var errorDescription: String? { if case let .message(s) = self { return s }; return nil }
}
public enum SubscriptionParser {
    // Only node definitions are imported. Remote rules, controller settings and scripts are discarded.
    public static func parse(_ text: String, source: Source) throws -> [ProxyNode] {
        guard text.utf8.count <= 10_000_000 else { throw ProxyError.message("订阅超过 10 MB") }
        let document: Any?
        do { document = try Yams.load(yaml: text) }
        catch { throw ProxyError.message("订阅 YAML 语法无效，已隐藏可能包含凭据的解析详情") }
        guard let doc = document as? [String: Any],
              let nodes = doc["proxies"] as? [[String: Any]], !nodes.isEmpty, nodes.count <= 5000 else {
            throw ProxyError.message("需要含 proxies 的 Clash YAML/JSON 订阅（最多 5000 个节点）；不支持 Base64 URI 列表或仅有 provider 的配置")
        }
        let supported: Set<String> = ["ss", "ssr", "vmess", "vless", "trojan", "hysteria", "hysteria2", "tuic", "socks5", "http", "snell", "anytls"]
        return try nodes.map { raw in
            guard let type = raw["type"] as? String, supported.contains(type),
                  let server = raw["server"] as? String, !server.isEmpty,
                  let port = raw["port"] as? Int, (1...65535).contains(port),
                  let original = raw["name"] as? String, !original.isEmpty else {
                throw ProxyError.message("订阅存在不支持或缺少 name/type/server/port 的节点")
            }
            try validate(raw)
            var identity = raw; identity.removeValue(forKey: "name")
            let data = try JSONSerialization.data(withJSONObject: identity, options: [.sortedKeys])
            let fingerprint = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            // A stable credential/config fingerprint deduplicates nodes across sources.
            let name = "\(original.prefix(70)) · \(fingerprint.prefix(12))"
            var payload = raw; payload["name"] = name
            return ProxyNode(id: fingerprint, name: name, sourceID: source.id, payload: payload)
        }
    }
    private static func validate(_ value: Any) throws {
        if let dict = value as? [String: Any] {
            for (key, item) in dict {
                let k = key.lowercased()
                if k.contains("path") && !["path"].contains(k) || ["dialer-proxy", "certificate", "private-key", "script", "plugin", "provider"].contains(k) {
                    throw ProxyError.message("不支持包含本地文件、插件或节点链引用的订阅")
                }
                // WebSocket / HTTP transport path is a URL path, not a filesystem reference.
                try validate(item)
            }
        } else if let list = value as? [Any] { for item in list { try validate(item) } }
    }
    public static func merge(_ batches: [[ProxyNode]]) -> [ProxyNode] {
        var seen = Set<String>()
        return batches.flatMap { $0 }.filter { seen.insert($0.id).inserted }
    }
}
