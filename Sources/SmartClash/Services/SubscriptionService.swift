import Foundation
import ProxyLogic
struct SubscriptionUpdate {
    var nodes: [ProxyNode]
    var warnings: [String]
}
enum SubscriptionService {
    static func update(_ sources: [Source], throughProxy: Bool, cacheDirectory: URL = Files.root) async throws -> SubscriptionUpdate {
        var batches: [[ProxyNode]] = []; var warnings: [String] = []
        for source in sources {
            try Task.checkCancellation()
            let cache = cacheDirectory.appendingPathComponent("subscription-\(source.id).yaml")
            do {
                if source.isLocal {
                    let data = try Data(contentsOf: cache)
                    batches.append(try SubscriptionParser.parse(String(decoding: data, as: UTF8.self), source: source))
                    continue
                }
                guard let url = URL(string: source.url), url.scheme == "https", url.user == nil, url.password == nil else {
                    throw ProxyError.message("订阅地址须为 HTTPS")
                }
                // URLs are passed as argv, never interpolated into a shell.
                let temp = cacheDirectory.appendingPathComponent("download-\(UUID()).tmp")
                defer { try? FileManager.default.removeItem(at: temp) }
                FileManager.default.createFile(atPath: temp.path, contents: Data(), attributes: [.posixPermissions: 0o600])
                var args = ["--fail", "--silent", "--show-error", "--location", "--max-redirs", "3", "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "8", "--max-time", "30", "--max-filesize", "10000000", "--user-agent", "clash.meta/SmartClash", "--output", temp.path]
                args += ["--noproxy", "", "--proxy", throughProxy ? "http://127.0.0.1:\(Configuration.proxyPort)" : ""]
                args += ["--url", source.url]
                let r = try await Command.run("/usr/bin/curl", args, cancellable: true)
                guard r.code == 0 else { throw ProxyError.message("下载失败（网络或 HTTP 错误）") }
                let data = try Data(contentsOf: temp)
                let nodes = try SubscriptionParser.parse(String(decoding: data, as: UTF8.self), source: source)
                try Files.write(data, to: cache); batches.append(nodes)
            } catch is CancellationError { throw CancellationError() }
            catch {
                if let data = try? Data(contentsOf: cache), let nodes = try? SubscriptionParser.parse(String(decoding: data, as: UTF8.self), source: source) {
                    batches.append(nodes); warnings.append("\(source.name)：更新失败，保留缓存")
                } else { warnings.append("\(source.name)：\(error.localizedDescription)") }
            }
        }
        let nodes = SubscriptionParser.merge(batches)
        guard !nodes.isEmpty else { throw ProxyError.message(warnings.isEmpty ? "请先添加订阅" : warnings.joined(separator: "\n")) }
        return SubscriptionUpdate(nodes: nodes, warnings: warnings)
    }
    static func cached(_ sources: [Source], cacheDirectory: URL = Files.root) -> [ProxyNode] {
        SubscriptionParser.merge(sources.compactMap { source in
            guard let data = try? Data(contentsOf: cacheDirectory.appendingPathComponent("subscription-\(source.id).yaml")) else { return nil }
            return try? SubscriptionParser.parse(String(decoding: data, as: UTF8.self), source: source)
        })
    }
}
