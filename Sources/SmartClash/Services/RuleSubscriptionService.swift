import Foundation
import ProxyLogic

enum RuleSubscriptionService {
    static let repository = "https://github.com/GMOogway/shadowrocket-rules"
    static func cached(at url: URL) -> RuleSnapshot? {
        guard let data = try? Data(contentsOf: url), let snapshot = try? JSONDecoder().decode(RuleSnapshot.self, from: data), snapshot.isComplete else { return nil }
        return snapshot
    }
    static func fetch(throughProxy: Bool) async throws -> RuleSnapshot {
        try await withThrowingTaskGroup(of: (RuleCategory, ConvertedRuleModule).self) { group in
            for category in RuleCategory.allCases {
                group.addTask { (category, try await download(category, throughProxy: throughProxy)) }
            }
            var modules: [RuleCategory: ConvertedRuleModule] = [:]
            for try await (category, module) in group { modules[category] = module }
            return RuleSnapshot(modules: modules)
        }
    }
    private static func download(_ category: RuleCategory, throughProxy: Bool) async throws -> ConvertedRuleModule {
        let urls = ["https://raw.githubusercontent.com/GMOogway/shadowrocket-rules/master/\(category.filename)",
                    "https://cdn.jsdelivr.net/gh/GMOogway/shadowrocket-rules@master/\(category.filename)"]
        let temporary = FileManager.default.temporaryDirectory.appendingPathComponent("smartclash-rule-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: temporary) }
        FileManager.default.createFile(atPath: temporary.path, contents: Data(), attributes: [.posixPermissions: 0o600])
        for (index, url) in urls.enumerated() {
            try Task.checkCancellation()
            let useProxy = throughProxy && index == 0
            let result = try await Command.run("/usr/bin/curl", ["--fail", "--silent", "--show-error", "--location", "--max-redirs", "3",
                "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "6", "--max-time", "30", "--max-filesize", "20000000",
                "--noproxy", "", "--proxy", useProxy ? "http://127.0.0.1:\(Configuration.proxyPort)" : "",
                "--output", temporary.path, "--url", url], cancellable: true)
            guard result.code == 0 else { continue }
            let text = try String(contentsOf: temporary, encoding: .utf8)
            // Parsing stays off the main actor even for the largest module.
            return try ShadowrocketRuleParser.parse(text, category: category)
        }
        throw ProxyError.message("\(category.filename) 下载失败；已保留最近成功的完整规则")
    }
}
