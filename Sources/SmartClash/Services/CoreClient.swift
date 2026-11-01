import Foundation
import ProxyLogic
final class CoreClient {
    let secret: String
    private let session: URLSession
    init(secret: String) {
        self.secret = secret
        let config = URLSessionConfiguration.ephemeral
        config.connectionProxyDictionary = [:]; config.timeoutIntervalForRequest = 4
        session = URLSession(configuration: config)
    }
    @discardableResult
    func request(_ path: String, method: String = "GET", body: [String: Any]? = nil) async throws -> [String: Any] {
        var req = URLRequest(url: URL(string: "http://127.0.0.1:\(Configuration.controllerPort)\(path)")!)
        req.httpMethod = method; req.setValue("Bearer \(secret)", forHTTPHeaderField: "Authorization")
        if let body { req.httpBody = try JSONSerialization.data(withJSONObject: body); req.setValue("application/json", forHTTPHeaderField: "Content-Type") }
        let (data, response) = try await session.data(for: req)
        guard let r = response as? HTTPURLResponse, (200..<300).contains(r.statusCode) else {
            throw ProxyError.message("内核控制接口调用失败：\(path)")
        }
        return (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }
    func select(_ name: String, group: String) async throws {
        try await request("/proxies/\(group)", method: "PUT", body: ["name": name])
    }
    func probe(port: Int) async throws -> ProbeResult {
        let start = Date()
        // curl uses a dedicated HTTP proxy listener, TLS verification stays enabled, no redirects.
        let r = try await Command.run("/usr/bin/curl", ["--silent", "--show-error", "--noproxy", "", "--proxy", "http://127.0.0.1:\(port)",
            "--connect-timeout", "4", "--max-time", "8", "--max-filesize", "131072", "--write-out", "\n%{http_code}",
            "--header", "Cache-Control: no-cache", "https://api.openai.com/v1/models"])
        let ms = Int(Date().timeIntervalSince(start) * 1000)
        guard r.code == 0, let cut = r.text.lastIndex(of: "\n"), let code = Int(r.text[r.text.index(after: cut)...]) else {
            return ProbeResult(health: .offline, milliseconds: ms, detail: "连接失败 / TLS 错误 / 超时")
        }
        return HealthPolicy.classify(code: code, body: String(r.text[..<cut]), elapsed: ms)
    }
}
