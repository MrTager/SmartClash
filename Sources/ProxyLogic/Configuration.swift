import Foundation
public enum Configuration {
    public static let proxyPort = 17890
    public static let probePort = 17891
    public static let controllerPort = 19090
    public static func make(nodes: [ProxyNode], secret: String, tun: Bool, routes: [ProxyNode]? = nil, extraProxies: [ProxyNode] = []) throws -> Data {
        guard !nodes.isEmpty else { throw ProxyError.message("请先添加并更新订阅") }
        let names = (routes ?? nodes).map(\.name)
        guard !names.isEmpty else { throw ProxyError.message("没有可用链路") }
        let obj: [String: Any] = [
            "mixed-port": proxyPort, "allow-lan": false, "bind-address": "127.0.0.1",
            "mode": "rule", "log-level": "warning", "ipv6": false,
            "external-controller": "127.0.0.1:\(controllerPort)", "secret": secret,
            "profile": ["store-selected": false], "find-process-mode": "off",
            "proxies": Dictionary((nodes + (extraProxies.isEmpty ? (routes ?? []) : extraProxies)).map { ($0.name, $0.payload) }, uniquingKeysWith: { _, new in new }).values.sorted { ($0["name"] as? String ?? "") < ($1["name"] as? String ?? "") },
            "proxy-groups": [
                ["name": "SMART", "type": "select", "proxies": names],
                ["name": "PROBE", "type": "select", "proxies": names]
            ],
            // Dedicated probe listener never changes the user's active routing group.
            "listeners": [["name": "openai-probe", "type": "http", "port": probePort,
                            "listen": "127.0.0.1", "proxy": "PROBE"]],
            "tun": ["enable": tun, "stack": "gvisor", "auto-route": true,
                    "auto-detect-interface": true, "dns-hijack": ["any:53"]],
            "dns": ["enable": true, "listen": "127.0.0.1:11053", "ipv6": false,
                    "enhanced-mode": "fake-ip", "fake-ip-range": "198.18.0.1/16",
                    "fake-ip-filter": ["*.lan", "*.local", "localhost"],
                    "default-nameserver": ["223.5.5.5", "1.1.1.1"],
                    "nameserver": ["https://dns.alidns.com/dns-query", "https://cloudflare-dns.com/dns-query"]],
            "rules": ["IP-CIDR,127.0.0.0/8,DIRECT,no-resolve", "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
                      "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve", "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve", "MATCH,SMART"]
        ]
        return try JSONSerialization.data(withJSONObject: obj, options: [.prettyPrinted, .sortedKeys])
    }
    public static func export(nodes: [ProxyNode]) throws -> Data {
        // JSON is valid YAML 1.2 and accepted by Mihomo/Clash clients; no local API secret is exported.
        let obj: [String: Any] = ["mixed-port": 7890, "allow-lan": false, "mode": "rule",
            "proxies": nodes.map(\.payload),
            "proxy-groups": [["name": "聚合节点", "type": "select", "proxies": nodes.map(\.name)]],
            "rules": ["MATCH,聚合节点"]]
        return try JSONSerialization.data(withJSONObject: obj, options: [.prettyPrinted, .sortedKeys])
    }
}
