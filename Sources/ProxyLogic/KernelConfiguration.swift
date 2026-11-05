import Foundation

/// Clash 节点字段 → sing-box / Xray 配置片段。
///
/// 订阅解析阶段保留的是 Clash 字段；这里只做协议与传输层映射，不做网络请求。
/// 无法可靠映射的节点会被明确标记为不支持，而不是生成一份错误的配置。
public enum KernelConversion {
    public static func supportError(kind: KernelKind, payload: [String: Any]) -> String? {
        let type = text(payload, "type") ?? ""
        guard text(payload, "server") != nil, number(payload, "port") != nil else {
            return "缺少服务器地址或端口"
        }
        if kind == .mihomo { return nil }
        if let error = transportError(payload) { return error }
        switch kind {
        case .singBox:
            switch type {
            case "ss":
                return payload["plugin"] == nil ? nil : "sing-box 不支持 Clash 内建的 SS 插件"
            case "vmess", "vless", "trojan", "hysteria", "hysteria2", "tuic", "anytls", "http":
                return nil
            case "socks5":
                return flag(payload, "tls") == true ? "sing-box 的 SOCKS5 出站不支持 TLS" : nil
            case "ssr":
                return "sing-box 不支持 ShadowsocksR"
            case "snell":
                return "sing-box 不支持 Snell"
            default:
                return "sing-box 不支持协议 \(type)"
            }
        case .xray:
            switch type {
            case "ss":
                return payload["plugin"] == nil ? nil : "Xray 不支持 Clash 内建的 SS 插件"
            case "vmess", "vless", "trojan", "socks5", "http":
                return nil
            case "hysteria", "hysteria2", "tuic", "anytls":
                return "Xray 不支持 \(type) 协议"
            case "ssr":
                return "Xray 不支持 ShadowsocksR"
            case "snell":
                return "Xray 不支持 Snell"
            default:
                return "Xray 不支持协议 \(type)"
            }
        case .mihomo:
            return nil
        }
    }

    // ------------------------------------------------------------ sing-box

    public static func toSingbox(node: ProxyNode) throws -> [String: Any] {
        let payload = node.payload
        if let error = supportError(kind: .singBox, payload: payload) { throw ProxyError.message(error) }
        let type = text(payload, "type") ?? ""
        var out: [String: Any] = [
            "tag": node.name,
            "server": text(payload, "server") ?? "",
            "server_port": number(payload, "port") ?? 0
        ]
        switch type {
        case "ss":
            out["type"] = "shadowsocks"
            out["method"] = text(payload, "cipher") ?? "aes-128-gcm"
            out["password"] = text(payload, "password") ?? ""
            if flag(payload, "udp-over-tcp") == true { out["udp_over_tcp"] = true }
        case "vmess":
            out["type"] = "vmess"
            out["uuid"] = text(payload, "uuid") ?? ""
            out["security"] = text(payload, "cipher") ?? "auto"
            out["alter_id"] = number(payload, "alterId") ?? 0
            if let tls = singboxTLS(payload, force: false) { out["tls"] = tls }
            if let transport = try singboxTransport(payload) { out["transport"] = transport }
        case "vless":
            out["type"] = "vless"
            out["uuid"] = text(payload, "uuid") ?? ""
            if let flow = text(payload, "flow") { out["flow"] = flow }
            if let tls = singboxTLS(payload, force: false) { out["tls"] = tls }
            if let transport = try singboxTransport(payload) { out["transport"] = transport }
        case "trojan":
            out["type"] = "trojan"
            out["password"] = text(payload, "password") ?? ""
            if let tls = singboxTLS(payload, force: true) { out["tls"] = tls }
            if let transport = try singboxTransport(payload) { out["transport"] = transport }
        case "hysteria":
            out["type"] = "hysteria"
            if let auth = text(payload, "auth-str") ?? text(payload, "auth_str") { out["auth_str"] = auth }
            if let value = mbps(payload, "up") { out["up_mbps"] = value }
            if let value = mbps(payload, "down") { out["down_mbps"] = value }
            if let obfs = text(payload, "obfs") { out["obfs"] = obfs }
            if let tls = singboxTLS(payload, force: true) { out["tls"] = tls }
        case "hysteria2":
            out["type"] = "hysteria2"
            out["password"] = text(payload, "password") ?? ""
            if let value = mbps(payload, "up") { out["up_mbps"] = value }
            if let value = mbps(payload, "down") { out["down_mbps"] = value }
            if let obfs = text(payload, "obfs") {
                out["obfs"] = ["type": obfs, "password": text(payload, "obfs-password") ?? ""]
            }
            if let tls = singboxTLS(payload, force: true) { out["tls"] = tls }
        case "tuic":
            out["type"] = "tuic"
            out["uuid"] = text(payload, "uuid") ?? ""
            out["password"] = text(payload, "password") ?? ""
            if let value = text(payload, "congestion-controller") { out["congestion_control"] = value }
            if let value = text(payload, "udp-relay-mode") { out["udp_relay_mode"] = value }
            if flag(payload, "reduce-rtt") == true { out["zero_rtt_handshake"] = true }
            if let tls = singboxTLS(payload, force: true) { out["tls"] = tls }
        case "anytls":
            out["type"] = "anytls"
            out["password"] = text(payload, "password") ?? ""
            if let tls = singboxTLS(payload, force: true) { out["tls"] = tls }
        case "http":
            out["type"] = "http"
            if let user = text(payload, "username") { out["username"] = user }
            if let password = text(payload, "password") { out["password"] = password }
            if let tls = singboxTLS(payload, force: false) { out["tls"] = tls }
        case "socks5":
            out["type"] = "socks"
            out["version"] = "5"
            if let user = text(payload, "username") { out["username"] = user }
            if let password = text(payload, "password") { out["password"] = password }
        default:
            throw ProxyError.message("sing-box 不支持协议 \(type)")
        }
        if let dialer = text(payload, "dialer-proxy") { out["detour"] = dialer }
        return out
    }

    // ---------------------------------------------------------------- Xray

    public static func xrayTag(_ name: String) -> String { "n:\(name)" }

    public static func toXray(node: ProxyNode) throws -> [String: Any] {
        let payload = node.payload
        if let error = supportError(kind: .xray, payload: payload) { throw ProxyError.message(error) }
        let type = text(payload, "type") ?? ""
        let server = text(payload, "server") ?? ""
        let port = number(payload, "port") ?? 0
        var out: [String: Any] = ["tag": xrayTag(node.name)]
        switch type {
        case "ss":
            out["protocol"] = "shadowsocks"
            out["settings"] = ["servers": [[
                "address": server, "port": port,
                "method": text(payload, "cipher") ?? "aes-128-gcm",
                "password": text(payload, "password") ?? ""
            ]]]
        case "vmess":
            out["protocol"] = "vmess"
            out["settings"] = ["vnext": [[
                "address": server, "port": port,
                "users": [[
                    "id": text(payload, "uuid") ?? "",
                    "alterId": number(payload, "alterId") ?? 0,
                    "security": text(payload, "cipher") ?? "auto",
                    "level": 0
                ]]
            ]]]
        case "vless":
            out["protocol"] = "vless"
            var user: [String: Any] = ["id": text(payload, "uuid") ?? "", "encryption": "none", "level": 0]
            if let flow = text(payload, "flow") { user["flow"] = flow }
            out["settings"] = ["vnext": [["address": server, "port": port, "users": [user]]]]
        case "trojan":
            out["protocol"] = "trojan"
            out["settings"] = ["servers": [[
                "address": server, "port": port,
                "password": text(payload, "password") ?? "", "level": 0
            ]]]
        case "socks5":
            out["protocol"] = "socks"
            var serverObject: [String: Any] = ["address": server, "port": port]
            if let user = text(payload, "username") {
                serverObject["users"] = [["user": user, "pass": text(payload, "password") ?? ""]]
            }
            out["settings"] = ["servers": [serverObject]]
        case "http":
            out["protocol"] = "http"
            var serverObject: [String: Any] = ["address": server, "port": port]
            if let user = text(payload, "username") {
                serverObject["users"] = [["user": user, "pass": text(payload, "password") ?? ""]]
            }
            out["settings"] = ["servers": [serverObject]]
        default:
            throw ProxyError.message("Xray 不支持协议 \(type)")
        }
        out["streamSettings"] = try xrayStream(payload, forceTLS: type == "trojan")
        return out
    }

    // ------------------------------------------------------------- helpers

    private static func text(_ p: [String: Any], _ key: String) -> String? {
        guard let value = p[key] as? String, !value.isEmpty else { return nil }
        return value
    }

    private static func number(_ p: [String: Any], _ key: String) -> Int? {
        guard let value = p[key] else { return nil }
        if let value = value as? Int { return value }
        if let value = value as? Double { return Int(value) }
        if let value = value as? NSNumber { return value.intValue }
        if let value = value as? String { return Int(value) }
        return nil
    }

    private static func flag(_ p: [String: Any], _ key: String) -> Bool? { p[key] as? Bool }

    private static func object(_ p: [String: Any], _ key: String) -> [String: Any]? { p[key] as? [String: Any] }

    private static func textList(_ value: Any?) -> [String]? {
        guard let list = value as? [Any] else { return nil }
        let values = list.compactMap { $0 as? String }
        return values.isEmpty ? nil : values
    }

    private static func network(_ p: [String: Any]) -> String { text(p, "network") ?? "tcp" }

    private static func protocolName(_ p: [String: Any]) -> String { text(p, "type") ?? "" }

    private static func transportError(_ p: [String: Any]) -> String? {
        switch protocolName(p) {
        case "vmess", "vless", "trojan":
            let net = network(p)
            return ["tcp", "ws", "grpc", "h2", "httpupgrade"].contains(net) ? nil : "不支持的传输方式 \(net)"
        default:
            return nil
        }
    }

    private static func firstHost(_ p: [String: Any], _ opts: [String: Any]?) -> String? {
        if let host = text(p, "servername") ?? text(p, "sni") { return host }
        guard let opts else { return nil }
        if let headers = opts["headers"] as? [String: Any] {
            for key in ["Host", "host"] {
                if let host = headers[key] as? String, !host.isEmpty { return host }
            }
        }
        if let host = opts["host"] as? String { return host }
        if let host = (opts["host"] as? [Any])?.first as? String { return host }
        return nil
    }

    private static func mbps(_ p: [String: Any], _ key: String) -> Int? {
        guard let value = number(p, key), value > 0 else { return nil }
        return value
    }

    private static func singboxTLS(_ p: [String: Any], force: Bool) -> [String: Any]? {
        let reality = object(p, "reality-opts")
        guard force || flag(p, "tls") == true || reality != nil else { return nil }
        var tls: [String: Any] = ["enabled": true]
        if let name = text(p, "servername") ?? text(p, "sni") { tls["server_name"] = name }
        if flag(p, "skip-cert-verify") == true { tls["insecure"] = true }
        if let alpn = textList(p["alpn"]) { tls["alpn"] = alpn }
        let fingerprint = text(p, "client-fingerprint") ?? text(p, "fingerprint")
        if let reality {
            var value: [String: Any] = ["enabled": true]
            if let key = text(reality, "public-key") { value["public_key"] = key }
            if let short = reality["short-id"] as? String { value["short_id"] = short }
            tls["reality"] = value
            tls["utls"] = ["enabled": true, "fingerprint": fingerprint ?? "chrome"]
        } else if let fingerprint {
            tls["utls"] = ["enabled": true, "fingerprint": fingerprint]
        }
        return tls
    }

    private static func singboxTransport(_ p: [String: Any]) throws -> [String: Any]? {
        let net = network(p)
        switch net {
        case "tcp":
            return nil
        case "ws":
            let opts = object(p, "ws-opts")
            var ws: [String: Any] = ["type": "ws", "path": (opts?["path"] as? String) ?? "/"]
            if let headers = opts?["headers"] as? [String: Any] {
                var map: [String: String] = [:]
                for (key, value) in headers {
                    if let value = value as? String { map[key] = value }
                }
                if !map.isEmpty { ws["headers"] = map }
            }
            return ws
        case "grpc":
            let service = object(p, "grpc-opts")?["grpc-service-name"] as? String ?? ""
            return ["type": "grpc", "service_name": service]
        case "h2":
            let opts = object(p, "h2-opts")
            var http: [String: Any] = ["type": "http", "path": (opts?["path"] as? String) ?? "/"]
            if let host = opts?["host"] as? [Any] {
                if !host.isEmpty { http["host"] = host }
            } else if let host = firstHost(p, opts) {
                http["host"] = [host]
            }
            return http
        case "httpupgrade":
            let opts = object(p, "http-upgrade-opts")
            var upgrade: [String: Any] = ["type": "httpupgrade", "path": (opts?["path"] as? String) ?? "/"]
            if let host = firstHost(p, opts) { upgrade["host"] = host }
            return upgrade
        default:
            throw ProxyError.message("不支持的传输方式 \(net)")
        }
    }

    private static func xrayReality(_ p: [String: Any]) -> [String: Any]? {
        guard let reality = object(p, "reality-opts") else { return nil }
        var value: [String: Any] = [:]
        if let name = text(p, "servername") ?? text(p, "sni") { value["serverName"] = name }
        if let key = text(reality, "public-key") { value["publicKey"] = key }
        if let short = reality["short-id"] as? String { value["shortId"] = short }
        if let fingerprint = text(p, "client-fingerprint") ?? text(p, "fingerprint") { value["fingerprint"] = fingerprint }
        if let spider = reality["spider-x"] as? String { value["spiderX"] = spider }
        return value
    }

    private static func xrayStream(_ p: [String: Any], forceTLS: Bool) throws -> [String: Any] {
        let net = network(p)
        let reality = xrayReality(p)
        let tlsEnabled = forceTLS || flag(p, "tls") == true || reality != nil
        guard ["tcp", "ws", "grpc", "h2", "httpupgrade"].contains(net) else {
            throw ProxyError.message("不支持的传输方式 \(net)")
        }
        var stream: [String: Any] = [
            "network": net,
            "security": reality != nil ? "reality" : (tlsEnabled ? "tls" : "none")
        ]
        if let reality {
            stream["realitySettings"] = reality
        } else if tlsEnabled {
            var tls: [String: Any] = [:]
            if let name = text(p, "servername") ?? text(p, "sni") { tls["serverName"] = name }
            if flag(p, "skip-cert-verify") == true { tls["allowInsecure"] = true }
            if let alpn = textList(p["alpn"]) { tls["alpn"] = alpn }
            if let fingerprint = text(p, "client-fingerprint") ?? text(p, "fingerprint") { tls["fingerprint"] = fingerprint }
            stream["tlsSettings"] = tls
        }
        switch net {
        case "ws":
            let opts = object(p, "ws-opts")
            var ws: [String: Any] = ["path": (opts?["path"] as? String) ?? "/"]
            if let host = firstHost(p, opts) { ws["headers"] = ["Host": host] }
            stream["wsSettings"] = ws
        case "grpc":
            let service = object(p, "grpc-opts")?["grpc-service-name"] as? String ?? ""
            stream["grpcSettings"] = ["serviceName": service]
        case "h2":
            let opts = object(p, "h2-opts")
            var h2: [String: Any] = ["path": (opts?["path"] as? String) ?? "/"]
            if let host = opts?["host"] as? [Any] {
                h2["host"] = host
            } else if let host = firstHost(p, opts) {
                h2["host"] = [host]
            }
            stream["h2Settings"] = h2
        case "httpupgrade":
            let opts = object(p, "http-upgrade-opts")
            var upgrade: [String: Any] = ["path": (opts?["path"] as? String) ?? "/"]
            if let host = firstHost(p, opts) { upgrade["host"] = host }
            stream["httpUpgradeSettings"] = upgrade
        default:
            break
        }
        if let dialer = text(p, "dialer-proxy") {
            stream["sockopt"] = ["dialerProxy": xrayTag(dialer)]
        }
        return stream
    }
}

/// 按内核生成完整配置。Mihomo 仍复用 `Configuration.make`。
public enum KernelConfiguration {
    public static let xrayAPIPort = 10085
    public static let xraySocksPort = 7893
    /// 官方 sing-box 规则集下载镜像。首次启动必须下载成功，失败会直接退出；
    /// raw.githubusercontent.com 在部分网络下不可达，因此默认走加速镜像并直连。
    public static let singboxRulesetMirror = "https://ghfast.top/"

    public struct Built {
        public let data: Data
        public let warnings: [String]
        public init(data: Data, warnings: [String] = []) { self.data = data; self.warnings = warnings }
    }

    public static func make(kind: KernelKind, nodes: [ProxyNode], secret: String, tun: Bool,
                            routes: [ProxyNode]? = nil, extraProxies: [ProxyNode] = [],
                            outboundInterface: String? = nil, preferredName: String? = nil,
                            trafficMode: TrafficMode = .rule, ruleProviderBaseURL: String? = nil) throws -> Built {
        switch kind {
        case .mihomo:
            let data = try Configuration.make(nodes: nodes, secret: secret, tun: tun, routes: routes,
                extraProxies: extraProxies, outboundInterface: outboundInterface, preferredName: preferredName,
                trafficMode: trafficMode, ruleProviderBaseURL: ruleProviderBaseURL)
            return Built(data: data)
        case .singBox:
            return try buildSingbox(nodes: nodes, secret: secret, tun: tun, routes: routes,
                extraProxies: extraProxies, outboundInterface: outboundInterface,
                preferredName: preferredName, trafficMode: trafficMode)
        case .xray:
            return try buildXray(nodes: nodes, tun: tun, routes: routes, extraProxies: extraProxies,
                outboundInterface: outboundInterface, preferredName: preferredName, trafficMode: trafficMode)
        }
    }

    public static func probeConfig(kind: KernelKind, node: ProxyNode, port: Int) throws -> Data {
        switch kind {
        case .mihomo:
            let object: [String: Any] = [
                "mixed-port": port, "allow-lan": false, "bind-address": "127.0.0.1",
                "mode": "rule", "log-level": "warning",
                "proxies": [node.payload],
                "rules": ["MATCH,\(node.name)"]
            ]
            return try serialize(object, error: "生成探测配置失败")
        case .singBox:
            let outbound = try KernelConversion.toSingbox(node: node)
            let config: [String: Any] = [
                "log": ["level": "warning"],
                "inbounds": [["type": "http", "tag": "probe-in", "listen": "127.0.0.1", "listen_port": port]],
                "outbounds": [outbound],
                "route": ["final": node.name]
            ]
            return try serialize(config, error: "生成探测配置失败")
        case .xray:
            let outbound = try KernelConversion.toXray(node: node)
            let config: [String: Any] = [
                "log": ["loglevel": "warning"],
                "inbounds": [["tag": "probe-in", "listen": "127.0.0.1", "port": port, "protocol": "http", "settings": [:]]],
                "outbounds": [outbound],
                "routing": [
                    "domainStrategy": "IPIfNonMatch",
                    "rules": [["type": "field", "inboundTag": ["probe-in"], "outboundTag": KernelConversion.xrayTag(node.name)]]
                ]
            ]
            return try serialize(config, error: "生成探测配置失败")
        }
    }

    // ------------------------------------------------------------ sing-box

    private static func buildSingbox(nodes: [ProxyNode], secret: String, tun: Bool, routes: [ProxyNode]?,
                                     extraProxies: [ProxyNode], outboundInterface: String?,
                                     preferredName: String?, trafficMode: TrafficMode) throws -> Built {
        let routeNodes = routes ?? nodes
        var routeKept = uniqueSupported(.singBox, routeNodes)
        if let preferredName, let index = routeKept.firstIndex(where: { $0.name == preferredName }) {
            routeKept.insert(routeKept.remove(at: index), at: 0)
        }
        if routeKept.isEmpty && trafficMode != .direct {
            throw ProxyError.message("当前内核不支持订阅中的节点协议，请更换订阅或内核")
        }
        var warnings: [String] = []
        let skipped = nodes.filter { !$0.supported(by: .singBox) }.count
        if skipped > 0 { warnings.append("当前内核（sing-box）不支持 \(skipped) 个节点协议，已从配置中跳过") }

        var outbounds: [[String: Any]] = []
        var seen = Set<String>()
        for node in nodes + extraProxies {
            guard seen.insert(node.name).inserted else { continue }
            guard KernelConversion.supportError(kind: .singBox, payload: node.payload) == nil else { continue }
            do { outbounds.append(try KernelConversion.toSingbox(node: node)) }
            catch { warnings.append("\(shortName(node.name))：\(error.localizedDescription)") }
        }

        let names = routeKept.map(\.name)
        var probeNames = names
        for node in nodes + extraProxies where KernelConversion.supportError(kind: .singBox, payload: node.payload) == nil {
            if !probeNames.contains(node.name) { probeNames.append(node.name) }
        }
        outbounds.append(["type": "selector", "tag": "SMART", "outbounds": names.isEmpty ? ["direct"] : names])
        outbounds.append(["type": "selector", "tag": "PROBE", "outbounds": probeNames.isEmpty ? ["direct"] : probeNames])
        outbounds.append(["type": "direct", "tag": "direct"])
        outbounds.append(["type": "block", "tag": "block"])

        var inbounds: [[String: Any]] = [
            ["type": "mixed", "tag": "mixed-in", "listen": "127.0.0.1", "listen_port": Configuration.proxyPort, "sniff": true],
            ["type": "http", "tag": "probe-in", "listen": "127.0.0.1", "listen_port": Configuration.probePort]
        ]
        if tun {
            inbounds.append(["type": "tun", "tag": "tun-in", "address": ["172.19.0.1/30"],
                             "auto_route": true, "strict_route": false, "stack": "gvisor", "sniff": true])
        }

        let ruleMode = trafficMode == .rule
        var rules: [[String: Any]] = [["inbound": ["probe-in"], "outbound": "PROBE"]]
        if ruleMode {
            rules.append(["ip_is_private": true, "outbound": "direct"])
            rules.append(["rule_set": ["cn-domains"], "outbound": "direct"])
            rules.append(["rule_set": ["cn-ips"], "outbound": "direct"])
        }
        var route: [String: Any] = [
            "auto_detect_interface": outboundInterface == nil,
            "default_domain_resolver": "dns-direct",
            "rules": rules,
            "final": trafficMode == .direct ? "direct" : "SMART"
        ]
        if let outboundInterface { route["default_interface"] = outboundInterface }
        if ruleMode {
            route["rule_set"] = [
                ["type": "remote", "tag": "cn-domains", "format": "binary",
                 "url": "\(singboxRulesetMirror)https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-cn.srs",
                 "download_detour": "direct", "update_interval": "1h"],
                ["type": "remote", "tag": "cn-ips", "format": "binary",
                 "url": "\(singboxRulesetMirror)https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-cn.srs",
                 "download_detour": "direct", "update_interval": "1h"]
            ]
        }
        var dns: [String: Any] = [
            "servers": [
                ["tag": "dns-proxy", "address": "https://1.1.1.1/dns-query",
                 "detour": trafficMode == .direct ? "direct" : "SMART"],
                ["tag": "dns-direct", "address": "223.5.5.5", "detour": "direct"]
            ],
            "final": trafficMode == .direct ? "dns-direct" : "dns-proxy",
            "strategy": "ipv4_only"
        ]
        var dnsRules: [[String: Any]] = [["ip_is_private": true, "server": "dns-direct"]]
        if ruleMode { dnsRules.append(["rule_set": ["cn-domains"], "server": "dns-direct"]) }
        dns["rules"] = dnsRules

        let config: [String: Any] = [
            "log": ["level": "warning", "timestamp": true],
            "inbounds": inbounds,
            "outbounds": outbounds,
            "route": route,
            "dns": dns,
            "experimental": [
                "clash_api": [
                    "external_controller": "127.0.0.1:\(Configuration.controllerPort)",
                    "secret": secret,
                    "default_mode": "rule"
                ]
            ]
        ]
        return Built(data: try serialize(config, error: "生成 sing-box 配置失败"), warnings: warnings)
    }

    // ---------------------------------------------------------------- Xray

    private static func buildXray(nodes: [ProxyNode], tun: Bool, routes: [ProxyNode]?,
                                  extraProxies: [ProxyNode], outboundInterface: String?,
                                  preferredName: String?, trafficMode: TrafficMode) throws -> Built {
        if tun { throw ProxyError.message("Xray 内核不支持 TUN，请改用 Mihomo 或 sing-box") }
        let routeNodes = routes ?? nodes
        var routeKept = uniqueSupported(.xray, routeNodes)
        if let preferredName, let index = routeKept.firstIndex(where: { $0.name == preferredName }) {
            routeKept.insert(routeKept.remove(at: index), at: 0)
        }
        if routeKept.isEmpty && trafficMode != .direct {
            throw ProxyError.message("当前内核不支持订阅中的节点协议，请更换订阅或内核")
        }
        var warnings: [String] = []
        let skipped = nodes.filter { !$0.supported(by: .xray) }.count
        if skipped > 0 { warnings.append("当前内核（Xray）不支持 \(skipped) 个节点协议，已从配置中跳过") }

        var outbounds: [[String: Any]] = []
        var seen = Set<String>()
        for node in nodes + extraProxies {
            guard seen.insert(node.name).inserted else { continue }
            guard KernelConversion.supportError(kind: .xray, payload: node.payload) == nil else { continue }
            do {
                var value = try KernelConversion.toXray(node: node)
                if let outboundInterface { applyXrayInterface(&value, interface: outboundInterface) }
                outbounds.append(value)
            } catch { warnings.append("\(shortName(node.name))：\(error.localizedDescription)") }
        }
        outbounds.append(["tag": "direct", "protocol": "freedom", "settings": [:]])
        outbounds.append(["tag": "block", "protocol": "blackhole", "settings": [:]])

        let finalTag: String
        if trafficMode == .direct {
            finalTag = "direct"
        } else if let first = routeKept.first {
            finalTag = KernelConversion.xrayTag(first.name)
        } else {
            finalTag = "direct"
        }

        var rules: [[String: Any]] = [["type": "field", "inboundTag": ["api"], "outboundTag": "api"]]
        if trafficMode == .rule {
            rules.append(["type": "field", "ip": ["geoip:private"], "outboundTag": "direct"])
            rules.append(["type": "field", "domain": ["geosite:cn"], "outboundTag": "direct"])
            rules.append(["type": "field", "ip": ["geoip:cn"], "outboundTag": "direct"])
        }
        rules.append(["type": "field", "network": "tcp,udp", "outboundTag": finalTag])

        let config: [String: Any] = [
            "log": ["loglevel": "warning"],
            "stats": [:],
            "api": ["tag": "api", "services": ["StatsService"]],
            "policy": ["system": [
                "statsInboundUplink": true, "statsInboundDownlink": true,
                "statsOutboundUplink": true, "statsOutboundDownlink": true
            ]],
            "dns": ["servers": ["223.5.5.5", "1.1.1.1"], "queryStrategy": "UseIPv4"],
            "inbounds": [
                ["tag": "http-in", "listen": "127.0.0.1", "port": Configuration.proxyPort,
                 "protocol": "http", "settings": [:],
                 "sniffing": ["enabled": true, "destOverride": ["http", "tls", "quic"]]],
                ["tag": "socks-in", "listen": "127.0.0.1", "port": xraySocksPort,
                 "protocol": "socks", "settings": ["udp": true],
                 "sniffing": ["enabled": true, "destOverride": ["http", "tls", "quic"]]],
                ["tag": "api", "listen": "127.0.0.1", "port": xrayAPIPort,
                 "protocol": "dokodemo-door", "settings": ["address": "127.0.0.1"]]
            ],
            "outbounds": outbounds,
            "routing": ["domainStrategy": "IPIfNonMatch", "rules": rules]
        ]
        return Built(data: try serialize(config, error: "生成 Xray 配置失败"), warnings: warnings)
    }

    private static func applyXrayInterface(_ outbound: inout [String: Any], interface: String) {
        var stream = outbound["streamSettings"] as? [String: Any] ?? [:]
        var sockopt = stream["sockopt"] as? [String: Any] ?? [:]
        sockopt["interface"] = interface
        stream["sockopt"] = sockopt
        outbound["streamSettings"] = stream
    }

    // ------------------------------------------------------------- helpers

    private static func uniqueSupported(_ kind: KernelKind, _ list: [ProxyNode]) -> [ProxyNode] {
        var seen = Set<String>()
        return list.filter { node in
            guard seen.insert(node.name).inserted else { return false }
            return node.supported(by: kind)
        }
    }

    private static func shortName(_ name: String) -> String {
        name.components(separatedBy: " · ").first ?? name
    }

    private static func serialize(_ object: [String: Any], error: String) throws -> Data {
        do { return try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]) }
        catch { throw ProxyError.message(error) }
    }
}

private extension ProxyNode {
    func supported(by kind: KernelKind) -> Bool {
        KernelConversion.supportError(kind: kind, payload: payload) == nil
    }
}
