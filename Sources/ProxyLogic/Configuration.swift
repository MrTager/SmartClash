import Foundation
public enum Configuration {
    public static let proxyPort = 7890
    public static let probePort = 7891
    public static let aggregatePort = 7892
    public static let controllerPort = 9090
    public static func make(nodes: [ProxyNode], secret: String, tun: Bool, routes: [ProxyNode]? = nil, extraProxies: [ProxyNode] = [], outboundInterface: String? = nil, preferredName: String? = nil, trafficMode: TrafficMode = .rule, ruleProviderBaseURL: String? = nil) throws -> Data {
        guard !nodes.isEmpty || trafficMode == .direct else { throw ProxyError.message("请先添加并更新订阅") }
        var names = (routes ?? nodes).map(\.name)
        if names.isEmpty && trafficMode == .direct { names = ["DIRECT"] }
        guard !names.isEmpty else { throw ProxyError.message("没有可用链路") }
        if let preferredName, let index = names.firstIndex(of: preferredName) { names.insert(names.remove(at: index), at: 0) }
        var seenProbe = Set<String>()
        let probeNames = (names + nodes.map(\.name)).filter { seenProbe.insert($0).inserted }
        var obj: [String: Any] = [
            "mixed-port": proxyPort, "allow-lan": false, "bind-address": "127.0.0.1",
            "mode": trafficMode.rawValue, "log-level": "warning", "ipv6": false,
            "external-controller": "127.0.0.1:\(controllerPort)", "secret": secret,
            "profile": ["store-selected": false], "find-process-mode": "off",
            "proxies": Dictionary((nodes + (extraProxies.isEmpty ? (routes ?? []) : extraProxies)).map { ($0.name, $0.payload) }, uniquingKeysWith: { _, new in new }).values.sorted { ($0["name"] as? String ?? "") < ($1["name"] as? String ?? "") },
            "proxy-groups": [
                ["name": "SMART", "type": "select", "proxies": names],
                ["name": "PROBE", "type": "select", "proxies": probeNames],
                ["name": "GLOBAL", "type": "select", "proxies": ["SMART"]]
            ],
            // Dedicated probe listener never changes the user's active routing group.
            "listeners": [["name": "openai-probe", "type": "http", "port": probePort,
                            "listen": "127.0.0.1", "proxy": "PROBE"]],
            "tun": ["enable": tun, "stack": "gvisor", "auto-route": true,
                    "auto-detect-interface": outboundInterface == nil, "dns-hijack": ["any:53"]],
            "dns": ["enable": true, "listen": "127.0.0.1:11053", "ipv6": false,
                    "enhanced-mode": "fake-ip", "fake-ip-range": "198.18.0.1/16",
                    "fake-ip-filter": ["*.lan", "*.local", "localhost"],
                    "default-nameserver": ["223.5.5.5", "119.29.29.29"],
                    "proxy-server-nameserver": ["223.5.5.5", "tcp://223.5.5.5", "119.29.29.29"],
                    "nameserver": ["https://dns.alidns.com/dns-query", "https://cloudflare-dns.com/dns-query"]],
            "rules": ["IP-CIDR,127.0.0.0/8,DIRECT,no-resolve", "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
                      "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve", "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve", "MATCH,SMART"]
        ]
        if let ruleProviderBaseURL {
            var providers: [String: Any] = [:]
            var rules = Array((obj["rules"] as! [String]).dropLast())
            rules += ["DOMAIN,localhost,DIRECT", "DOMAIN-SUFFIX,local,DIRECT", "DOMAIN-SUFFIX,lan,DIRECT"]
            for category in RuleCategory.allCases {
                for behavior in ["domain", "classical"] {
                    let name = category.provider(behavior)
                    providers[name] = ["type": "http", "behavior": behavior, "format": "yaml", "interval": 3600,
                                       "url": "\(ruleProviderBaseURL)/\(name).yaml", "path": "./rule-providers/\(name).yaml", "proxy": "DIRECT"]
                    rules.append("RULE-SET,\(name),\(category.policy)")
                }
            }
            rules.append("MATCH,SMART")
            obj["rules"] = rules; obj["rule-providers"] = providers
        }
        if let outboundInterface { obj["interface-name"] = outboundInterface }
        return try JSONSerialization.data(withJSONObject: obj, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
    }
    public static func export(nodes: [ProxyNode]) throws -> Data {
        // JSON is valid YAML 1.2 and accepted by Mihomo/Clash clients; no local API secret is exported.
        let obj: [String: Any] = ["mixed-port": 7890, "allow-lan": false, "mode": "rule",
            "proxies": nodes.map(\.payload),
            "proxy-groups": [["name": "聚合节点", "type": "select", "proxies": nodes.map(\.name)]],
            "rules": ["MATCH,聚合节点"]]
        return try JSONSerialization.data(withJSONObject: obj, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
    }
}
