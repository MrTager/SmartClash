import Foundation
import ProxyLogic

enum RuleSubscriptionService {
    static let repository = RuleSnapshot.repository
    static func cached(at url: URL) -> RuleSnapshot? {
        guard let data = try? Data(contentsOf: url), let snapshot = try? JSONDecoder().decode(RuleSnapshot.self, from: data),
              snapshot.source == repository, snapshot.isComplete else { return nil }
        return snapshot
    }
    static func fetch(throughProxy: Bool) async throws -> RuleSnapshot {
        try await withThrowingTaskGroup(of: ConvertedRuleModule.self) { group in
            for filename in ["direct.yaml", "custom-direct.yaml", "cncidr.yaml"] {
                group.addTask { try await download(filename, throughProxy: throughProxy) }
            }
            var direct = ConvertedRuleModule()
            for try await module in group {
                direct.domains += module.domains
                direct.classical += module.classical
            }
            direct.domains = Array(Set(direct.domains)).sorted()
            direct.classical = Array(Set(direct.classical)).sorted()
            let snapshot = RuleSnapshot(modules: [.direct: direct], source: repository)
            guard snapshot.isComplete else { throw ProxyError.message("规则文件不完整，已保留最近成功的完整规则") }
            return snapshot
        }
    }
    private static func download(_ filename: String, throughProxy: Bool) async throws -> ConvertedRuleModule {
        let urls = ["https://raw.githubusercontent.com/MrTager/SmartClash-Rules/main/rules/\(filename)",
                    "https://cdn.jsdelivr.net/gh/MrTager/SmartClash-Rules@main/rules/\(filename)"]
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
            return try SmartClashRuleParser.parse(text, ipcidr: filename == "cncidr.yaml")
        }
        throw ProxyError.message("\(filename) 下载失败；已保留最近成功的完整规则")
    }
}
