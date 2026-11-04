import Foundation
import Darwin

public enum RuleCategory: String, Codable, CaseIterable, Sendable {
    case reject, proxy, direct
    public var filename: String { "sr_\(rawValue)_list.module" }
    public var policy: String {
        switch self { case .reject: return "REJECT"; case .proxy: return "SMART"; case .direct: return "DIRECT" }
    }
    public func provider(_ behavior: String) -> String { "sr-\(rawValue)-\(behavior)" }
}

public struct ConvertedRuleModule: Codable, Equatable, Sendable {
    public var domains: [String]
    public var classical: [String]
    public var skipped: Int
    public var count: Int { domains.count + classical.count }
    public init(domains: [String] = [], classical: [String] = [], skipped: Int = 0) {
        self.domains = domains; self.classical = classical; self.skipped = skipped
    }
}

public struct RuleSnapshot: Codable, Equatable, Sendable {
    public static let updateInterval: TimeInterval = 3600
    public var modules: [RuleCategory: ConvertedRuleModule]
    public var updatedAt: Date
    public var count: Int { modules.values.reduce(0) { $0 + $1.count } }
    public var skipped: Int { modules.values.reduce(0) { $0 + $1.skipped } }
    public var isComplete: Bool { RuleCategory.allCases.allSatisfy { (modules[$0]?.count ?? 0) > 0 } }
    public func isDue(at now: Date = Date()) -> Bool { now.timeIntervalSince(updatedAt) >= Self.updateInterval }
    public init(modules: [RuleCategory: ConvertedRuleModule], updatedAt: Date = Date()) {
        self.modules = modules; self.updatedAt = updatedAt
    }
    public func providerPayloads() throws -> [String: Data] {
        var result: [String: Data] = [:]
        for category in RuleCategory.allCases {
            let module = modules[category] ?? ConvertedRuleModule()
            for (behavior, entries) in [("domain", module.domains), ("classical", module.classical)] {
                // A harmless placeholder keeps provider names stable before the first download.
                let payload = entries.isEmpty ? [behavior == "domain" ? "smartclash-empty.invalid" : "DOMAIN,smartclash-empty.invalid"] : entries
                // Mihomo's rule-provider reader requires a YAML `payload:` header.
                let lines = payload.map { value in
                    "  - \"" + value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
                }
                result[category.provider(behavior)] = Data(("payload:\n" + lines.joined(separator: "\n") + "\n").utf8)
            }
        }
        return result
    }
}

public enum ShadowrocketRuleParser {
    public static func parse(_ text: String, category: RuleCategory) throws -> ConvertedRuleModule {
        guard text.utf8.count <= 20_000_000 else { throw ProxyError.message("规则模块超过 20 MB") }
        var result = ConvertedRuleModule()
        var seen = Set<String>()
        var inRules = false
        var declaredCount: Int?
        var rowCount = 0
        let expected = category == .proxy ? "PROXY" : category.policy
        for rawLine in text.split(whereSeparator: \.isNewline) {
            let line = rawLine.trimmingCharacters(in: .whitespacesAndNewlines).replacingOccurrences(of: "\u{feff}", with: "")
            if line.hasPrefix("#!desc="), let range = line.range(of: "Rules:") {
                declaredCount = Int(line[range.upperBound...].prefix(while: \.isNumber))
            }
            if line.isEmpty || line.hasPrefix("#") || line.hasPrefix(";") { continue }
            if line.hasPrefix("[") {
                guard line == "[Rule]" else { throw ProxyError.message("规则模块包含不支持的配置节") }
                inRules = true; continue
            }
            guard inRules else { throw ProxyError.message("规则模块缺少 [Rule] 配置节") }
            rowCount += 1
            if rowCount % 1024 == 0 { try Task.checkCancellation() }
            // These HTTP-layer rules cannot be represented by Mihomo routing rules.
            if line.hasPrefix("URL-REGEX,") || line.hasPrefix("USER-AGENT,") { result.skipped += 1; continue }
            let fields = line.split(separator: ",", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespaces) }
            guard fields.count == 3 || fields.count == 4, fields[2] == expected else { result.skipped += 1; continue }
            let kind = fields[0], value = fields[1].lowercased()
            guard !value.isEmpty else { throw ProxyError.message("规则内容为空") }
            var entry: String
            switch kind {
            case "DOMAIN", "DOMAIN-SUFFIX", "DOMAIN-KEYWORD":
                guard fields.count == 3, value.utf8.allSatisfy({ (33...126).contains($0) && ![34, 39, 44, 47, 92].contains($0) }) else {
                    throw ProxyError.message("域名规则格式无效")
                }
                if kind == "DOMAIN-KEYWORD" { entry = "DOMAIN-KEYWORD,\(value)" }
                else { entry = kind == "DOMAIN-SUFFIX" ? "+.\(value.trimmingCharacters(in: CharacterSet(charactersIn: ".")))" : value }
            case "IP-CIDR", "IP-CIDR6":
                let parts = value.split(separator: "/")
                var address = in6_addr()
                let family = kind == "IP-CIDR" ? AF_INET : AF_INET6
                guard parts.count == 2, let prefix = Int(parts[1]), (0...(kind == "IP-CIDR" ? 32 : 128)).contains(prefix),
                      String(parts[0]).withCString({ inet_pton(family, $0, &address) }) == 1,
                      fields.count == 3 || fields[3] == "no-resolve" else { throw ProxyError.message("IP 规则格式无效") }
                entry = "\(kind),\(value)" + (fields.count == 4 ? ",no-resolve" : "")
            default: throw ProxyError.message("规则模块包含不支持的类型：\(kind.prefix(40))")
            }
            guard seen.insert(entry).inserted else { continue }
            if kind == "DOMAIN" || kind == "DOMAIN-SUFFIX" { result.domains.append(entry) }
            else { result.classical.append(entry) }
        }
        if let declaredCount, declaredCount != rowCount { throw ProxyError.message("规则模块不完整，已保留旧版本") }
        guard inRules, result.count > 0 else { throw ProxyError.message("规则模块没有可用规则，已保留旧版本") }
        return result
    }
}
