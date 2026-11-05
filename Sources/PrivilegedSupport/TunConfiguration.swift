import Foundation
import ProxyLogic

public enum TunConfiguration {
    // Only the app's generated configuration is accepted by the root service.
    // Executable paths, controller addresses and filesystem destinations are never supplied over XPC.
    public static func validate(_ data: Data) throws { try validate(data, kernel: .mihomo) }

    public static func validate(_ data: Data, kernel: KernelKind) throws {
        switch kernel {
        case .mihomo: try validateMihomo(data)
        case .singBox: try validateSingbox(data)
        case .xray: throw ProxyError.message("Xray 内核不支持 TUN")
        }
    }

    private static func validateMihomo(_ data: Data) throws {
        guard data.count <= 10_000_000,
              let config = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw invalid() }
        let keys: Set<String> = ["mixed-port", "allow-lan", "bind-address", "mode", "log-level", "ipv6",
            "external-controller", "secret", "profile", "find-process-mode", "proxies", "proxy-groups",
            "listeners", "tun", "dns", "rules", "rule-providers", "interface-name"]
        guard Set(config.keys).isSubset(of: keys),
              config["mixed-port"] as? Int == Configuration.proxyPort,
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
        // These sections never need a local file reference in generated configurations.
        for key in ["dns", "proxy-groups", "profile", "tun", "listeners"] {
            if let value = config[key] { try rejectFileReferences(value) }
        }
    }
    private static func validateSingbox(_ data: Data) throws {
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
                      inbound["listen_port"] as? Int == Configuration.proxyPort else { throw invalid() }
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
    private static func invalid() -> ProxyError { .message("TUN 配置不符合后台服务的安全限制，请重新生成配置") }
}
