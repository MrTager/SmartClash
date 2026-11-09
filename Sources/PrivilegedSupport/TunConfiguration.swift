import Foundation
import ProxyLogic

public enum TunConfiguration {
    // Only the app's generated configuration is accepted by the root service.
    // Executable paths, controller addresses and filesystem destinations are never supplied over XPC.
    public static func validate(_ data: Data) throws { try validate(data, kernel: .mihomo) }

    public static func validate(_ data: Data, kernel: KernelKind, port: Int = Configuration.proxyPort) throws {
        guard Configuration.validateProxyPort(port) == nil else { throw invalid() }
        switch kernel {
        case .mihomo: try validateMihomo(data, port: port)
        case .singBox: try validateSingbox(data, port: port)
        case .xray: throw ProxyError.message("Xray 内核不支持 TUN")
        }
    }

    private static func validateMihomo(_ data: Data, port: Int) throws {
        guard data.count <= 10_000_000,
              let config = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw invalid() }
        let keys: Set<String> = ["mixed-port", "allow-lan", "bind-address", "mode", "log-level", "ipv6",
            "external-controller", "secret", "profile", "find-process-mode", "proxies", "proxy-groups",
            "listeners", "tun", "dns", "rules", "rule-providers", "interface-name"]
        guard Set(config.keys).isSubset(of: keys),
              config["mixed-port"] as? Int == port,
              config["allow-lan"] as? Bool == false, config["bind-address"] as? String == "127.0.0.1",
              config["external-controller"] as? String == "127.0.0.1:\(Configuration.controllerPort)",
              let secret = config["secret"] as? String, secret.utf8.count >= 16,
              ["rule", "global", "direct"].contains(config["mode"] as? String ?? ""),
              let tun = config["tun"] as? [String: Any], tun["enable"] as? Bool == true,
              Set(tun.keys).isSubset(of: ["enable", "stack", "auto-route", "auto-detect-interface", "dns-hijack"]),
              let dns = config["dns"] as? [String: Any], dns["listen"] as? String == "127.0.0.1:11053",
              let listeners = config["listeners"] as? [[String: Any]], listeners.count == 1,
              listeners[0]["port"] as? Int == Configuration.probePort,
              listeners[0]["listen"] as? String == "127.0.0.1",
              Set(listeners[0].keys).isSubset(of: ["name", "type", "port", "listen", "proxy"]),
              let proxies = config["proxies"] as? [[String: Any]], proxies.count <= 5000 else { throw invalid() }
        for proxy in proxies { try validateProxy(proxy) }
        if let providers = config["rule-providers"] as? [String: [String: Any]] {
            let names = Set(RuleCategory.allCases.flatMap { [$0.provider("domain"), $0.provider("classical")] })
            guard Set(providers.keys).isSubset(of: names) else { throw invalid() }
            for (name, provider) in providers {
                guard provider["type"] as? String == "http",
                      provider["path"] as? String == "./rule-providers/\(name).yaml",
                      let text = provider["url"] as? String, let url = URL(string: text),
                      url.scheme == "http", url.host == "127.0.0.1", url.port == Configuration.aggregatePort,
                      url.user == nil, url.password == nil,
                      Set(provider.keys).isSubset(of: ["type", "behavior", "format", "interval", "url", "path", "proxy"]) else { throw invalid() }
            }
        } else if config["rule-providers"] != nil { throw invalid() }
        try strictMihomo(config)
        // These sections never need a local file reference in generated configurations.
        for key in ["dns", "proxy-groups", "profile", "tun", "listeners"] {
            if let value = config[key] { try rejectFileReferences(value) }
        }
    }
    private static func validateSingbox(_ data: Data, port: Int) throws {
        guard data.count <= 10_000_000,
              let config = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw invalid() }
        let keys: Set<String> = ["log", "inbounds", "outbounds", "route", "dns", "experimental"]
        guard Set(config.keys).isSubset(of: keys),
              let inbounds = config["inbounds"] as? [[String: Any]], !inbounds.isEmpty,
              let outbounds = config["outbounds"] as? [[String: Any]], outbounds.count <= 5002,
              let route = config["route"] as? [String: Any],
              let rules = route["rules"] as? [[String: Any]], rules.count <= 64,
              ["SMART", "direct"].contains(route["final"] as? String ?? ""),
              let dns = config["dns"] as? [String: Any],
              let servers = dns["servers"] as? [[String: Any]], !servers.isEmpty,
              let experimental = config["experimental"] as? [String: Any],
              let clash = experimental["clash_api"] as? [String: Any],
              clash["external_controller"] as? String == "127.0.0.1:\(Configuration.controllerPort)",
              let secret = clash["secret"] as? String, secret.utf8.count >= 16 else { throw invalid() }
        var mixed = false, probe = false, tun = false
        for inbound in inbounds {
            switch inbound["type"] as? String {
            case "mixed":
                guard inbound["listen"] as? String == "127.0.0.1",
                      inbound["listen_port"] as? Int == port else { throw invalid() }
                mixed = true
            case "http":
                guard inbound["listen"] as? String == "127.0.0.1",
                      inbound["listen_port"] as? Int == Configuration.probePort else { throw invalid() }
                probe = true
            case "tun":
                guard inbound["auto_route"] as? Bool == true,
                      let address = inbound["address"] as? [String], address == ["172.19.0.1/30"] else { throw invalid() }
                tun = true
            default:
                throw invalid()
            }
            try rejectSingboxReferences(inbound)
        }
        guard mixed, probe, tun else { throw invalid() }
        let allowed: Set<String> = ["shadowsocks", "vmess", "vless", "trojan", "hysteria", "hysteria2",
                                    "tuic", "anytls", "http", "socks", "direct", "block", "selector"]
        for outbound in outbounds {
            guard let type = outbound["type"] as? String, allowed.contains(type) else { throw invalid() }
            try rejectSingboxReferences(outbound)
        }
        try strictSingbox(config)
    }

    private static func rejectSingboxReferences(_ value: Any, transport: Bool = false) throws {
        if let dict = value as? [String: Any] {
            for (key, child) in dict {
                let lower = key.lowercased()
                guard !(lower.contains("path") && !(lower == "path" && transport)),
                      !["certificate", "private_key", "private-key", "script", "plugin", "provider", "cache_file"].contains(lower) else { throw invalid() }
                try rejectSingboxReferences(child, transport: transport || lower == "transport")
            }
        } else if let items = value as? [Any] {
            for item in items { try rejectSingboxReferences(item, transport: transport) }
        }
    }

    private static func validateProxy(_ proxy: [String: Any]) throws {
        var checked = proxy
        if proxy["type"] as? String == "ss", proxy["plugin"] as? String == "obfs",
           let options = proxy["plugin-opts"] as? [String: String],
           Set(options.keys).isSubset(of: ["mode", "host"]), ["http", "tls"].contains(options["mode"] ?? "") {
            checked.removeValue(forKey: "plugin")
        }
        try rejectFileReferences(checked)
    }
    private static func rejectFileReferences(_ value: Any, transport: Bool = false) throws {
        if let dict = value as? [String: Any] {
            for (key, child) in dict {
                let key = key.lowercased()
                guard !(key.contains("path") && !(key == "path" && transport)),
                      !["certificate", "private-key", "script", "plugin", "provider"].contains(key) else { throw invalid() }
                try rejectFileReferences(child, transport: ["ws-opts", "http-opts", "h2-opts"].contains(key))
            }
        } else if let items = value as? [Any] {
            for item in items { try rejectFileReferences(item, transport: transport) }
        }
    }
    private static func fields(_ value: Any?, _ keys: Set<String>) throws -> [String: Any] {
        guard let object = value as? [String: Any], Set(object.keys).isSubset(of: keys) else { throw invalid() }
        return object
    }
    private static func scalars(_ object: [String: Any], except nested: Set<String> = []) throws {
        for (key, value) in object where !nested.contains(key) {
            if value is String || value is NSNumber { continue }
            if let list = value as? [Any], list.allSatisfy({ $0 is String || $0 is NSNumber }) { continue }
            throw invalid()
        }
    }
    private static func strictMihomo(_ config: [String: Any]) throws {
        _ = try fields(config["profile"], ["store-selected"])
        let dns = try fields(config["dns"], ["enable", "listen", "ipv6", "enhanced-mode", "fake-ip-range", "fake-ip-filter", "default-nameserver", "proxy-server-nameserver", "nameserver"])
        try scalars(dns)
        guard let groups = config["proxy-groups"] as? [[String: Any]], groups.count == 3 else { throw invalid() }
        for group in groups { _ = try fields(group, ["name", "type", "proxies"]); try scalars(group); guard group["type"] as? String == "select" else { throw invalid() } }
        guard let rules = config["rules"] as? [String], rules.count <= 128 else { throw invalid() }
        for proxy in config["proxies"] as? [[String: Any]] ?? [] {
            // 根服务只接收无文件、无插件执行能力的有限节点 schema；未知字段默认拒绝。
            let common: Set<String> = ["name", "type", "server", "port", "cipher", "password", "uuid", "alterId", "udp", "tls", "servername", "sni", "skip-cert-verify", "network", "alpn", "client-fingerprint", "fingerprint", "dialer-proxy", "flow", "username", "auth-str", "auth_str", "up", "down", "obfs", "obfs-password", "congestion-controller", "udp-relay-mode", "reduce-rtt", "udp-over-tcp", "protocol", "protocol-param", "obfs-param", "version", "ws-opts", "http-opts", "h2-opts", "grpc-opts", "http-upgrade-opts", "reality-opts", "plugin", "plugin-opts"]
            _ = try fields(proxy, common)
            guard ["ss", "ssr", "vmess", "vless", "trojan", "hysteria", "hysteria2", "tuic", "socks5", "http", "snell", "anytls"].contains(proxy["type"] as? String ?? ""),
                  let port = proxy["port"] as? Int, (1...65535).contains(port), proxy["server"] is String else { throw invalid() }
            let nested: Set<String> = ["ws-opts", "http-opts", "h2-opts", "grpc-opts", "http-upgrade-opts", "reality-opts", "plugin-opts"]
            try scalars(proxy, except: nested)
            for name in nested where proxy[name] != nil {
                let keys: Set<String>
                switch name {
                case "reality-opts": keys = ["public-key", "short-id"]
                case "grpc-opts": keys = ["grpc-service-name"]
                case "plugin-opts": keys = ["mode", "host"]
                default: keys = ["path", "headers", "host", "method"]
                }
                let object = try fields(proxy[name], keys); try scalars(object, except: ["headers"])
                if let headers = object["headers"] { guard let map = headers as? [String: String], map.count <= 64 else { throw invalid() } }
            }
        }
        try scalars(try fields(config["tun"], ["enable", "stack", "auto-route", "auto-detect-interface", "dns-hijack"]))
        for provider in (config["rule-providers"] as? [String: [String: Any]] ?? [:]).values { try scalars(provider) }
    }
    private static func strictSingbox(_ config: [String: Any]) throws {
        try scalars(try fields(config["log"], ["level", "timestamp"]))
        let experimental = try fields(config["experimental"], ["clash_api"])
        try scalars(try fields(experimental["clash_api"], ["external_controller", "secret", "default_mode"]))
        for inbound in config["inbounds"] as? [[String: Any]] ?? [] {
            let keys: Set<String> = inbound["type"] as? String == "tun"
                ? ["type", "tag", "address", "auto_route", "strict_route", "stack", "sniff"]
                : ["type", "tag", "listen", "listen_port", "sniff"]
            try scalars(try fields(inbound, keys))
        }
        let route = try fields(config["route"], ["auto_detect_interface", "default_domain_resolver", "rules", "final", "default_interface", "rule_set"])
        try scalars(route, except: ["rules", "rule_set"])
        for rule in route["rules"] as? [[String: Any]] ?? [] { try scalars(try fields(rule, ["inbound", "outbound", "ip_is_private", "rule_set"])) }
        for rule in route["rule_set"] as? [[String: Any]] ?? [] {
            try scalars(try fields(rule, ["type", "tag", "format", "url", "download_detour", "update_interval"]))
            let urls = ["geosite-cn.srs", "geoip-cn.srs"]
            guard rule["type"] as? String == "remote", rule["format"] as? String == "binary",
                  let text = rule["url"] as? String,
                  text == KernelConfiguration.singboxRulesetMirror + "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/" + urls[0] || text == KernelConfiguration.singboxRulesetMirror + "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/" + urls[1] else { throw invalid() }
        }
        let dns = try fields(config["dns"], ["servers", "rules", "final", "strategy"])
        try scalars(dns, except: ["servers", "rules"])
        for server in dns["servers"] as? [[String: Any]] ?? [] { try scalars(try fields(server, ["tag", "address", "detour"])) }
        for rule in dns["rules"] as? [[String: Any]] ?? [] { try scalars(try fields(rule, ["ip_is_private", "server", "rule_set"])) }
        for outbound in config["outbounds"] as? [[String: Any]] ?? [] {
            let nested: Set<String> = ["tls", "transport", "obfs"]
            _ = try fields(outbound, ["type", "tag", "server", "server_port", "method", "password", "uuid", "security", "alter_id", "flow", "auth_str", "up_mbps", "down_mbps", "obfs", "congestion_control", "udp_relay_mode", "zero_rtt_handshake", "username", "version", "detour", "outbounds", "udp_over_tcp", "tls", "transport"])
            try scalars(outbound, except: nested)
            if let value = outbound["tls"] {
                let tls = try fields(value, ["enabled", "server_name", "insecure", "alpn", "reality", "utls"])
                try scalars(tls, except: ["reality", "utls"])
                if let reality = tls["reality"] { try scalars(try fields(reality, ["enabled", "public_key", "short_id"])) }
                if let utls = tls["utls"] { try scalars(try fields(utls, ["enabled", "fingerprint"])) }
            }
            if let value = outbound["transport"] {
                let transport = try fields(value, ["type", "path", "headers", "host", "service_name"])
                try scalars(transport, except: ["headers"])
                if let headers = transport["headers"] { guard headers is [String: String] else { throw invalid() } }
            }
            if let value = outbound["obfs"], !(value is String) { try scalars(try fields(value, ["type", "password"])) }
        }
    }
    private static func invalid() -> ProxyError { .message("TUN 配置不符合后台服务的安全限制，请重新生成配置") }
}
