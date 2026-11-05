import Foundation
import ProxyLogic
final class CoreClient: @unchecked Sendable {
    let secret: String
    private let session: URLSession
    init(secret: String, session suppliedSession: URLSession? = nil) {
        self.secret = secret
        let config = URLSessionConfiguration.ephemeral
        config.connectionProxyDictionary = [:]; config.timeoutIntervalForRequest = 4
        session = suppliedSession ?? URLSession(configuration: config)
    }
    @discardableResult
    func request(_ path: String, method: String = "GET", body: [String: Any]? = nil, timeout: TimeInterval = 4) async throws -> [String: Any] {
        try Task.checkCancellation()
        var req = URLRequest(url: URL(string: "http://127.0.0.1:\(Configuration.controllerPort)\(path)")!)
        req.timeoutInterval = timeout
        req.httpMethod = method; req.setValue("Bearer \(secret)", forHTTPHeaderField: "Authorization")
        if let body { req.httpBody = try JSONSerialization.data(withJSONObject: body); req.setValue("application/json", forHTTPHeaderField: "Content-Type") }
        let (data, response) = try await session.data(for: req)
        try Task.checkCancellation()
        guard let r = response as? HTTPURLResponse, (200..<300).contains(r.statusCode) else {
            throw ProxyError.message("内核控制接口调用失败：\(path)")
        }
        return (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }
    func select(_ name: String, group: String) async throws {
        try await request("/proxies/\(group)", method: "PUT", body: ["name": name])
    }
    func setTrafficMode(_ mode: TrafficMode) async throws {
        if mode == .global { try await select("SMART", group: "GLOBAL") }
        try await request("/configs", method: "PATCH", body: ["mode": mode.rawValue])
        let current = try await request("/configs")
        guard current["mode"] as? String == mode.rawValue else { throw ProxyError.message("内核未确认流量模式，已尝试恢复原模式") }
    }
    func closeConnections() async throws { try await request("/connections", method: "DELETE") }
    func updateRuleProviders() async throws {
        for category in RuleCategory.allCases {
            for behavior in ["domain", "classical"] {
                try await request("/providers/rules/\(category.provider(behavior))", method: "PUT", timeout: 30)
            }
        }
    }
    func delay(_ name: String, timeout: Int = 5000) async -> NodeLatency {
        do {
            let response = try await request(DelayEndpoint.path(name: name, timeout: timeout), timeout: Double(timeout) / 1000 + 2)
            guard let milliseconds = response["delay"] as? Int, milliseconds >= 0 else {
                return NodeLatency(milliseconds: nil, failure: "未返回有效延迟")
            }
            return NodeLatency(milliseconds: milliseconds)
        } catch {
            return NodeLatency(milliseconds: nil, failure: Task.isCancelled ? "已取消" : "超时或不可达")
        }
    }
    func internetProbe(port: Int) async throws -> ProbeResult {
        let start = Date()
        let result = try await Command.run("/usr/bin/curl", ["--silent", "--show-error", "--noproxy", "", "--proxy", "http://127.0.0.1:\(port)",
            "--connect-timeout", "3", "--max-time", "6", "--output", "/dev/null", "--write-out", "\n%{http_code}",
            "--header", "Cache-Control: no-cache", DelayEndpoint.target], cancellable: true)
        // Command.run merges stdout and stderr into one pipe, so curl's --show-error
        // text sits next to the status code. Parse the code from its own line and keep
        // the transport error: "HTTP 0" alone cannot distinguish DNS failure, a dead
        // exit node and a proxy that refused the CONNECT.
        var code = 0
        var diagnostics: [String] = []
        for line in result.text.components(separatedBy: .newlines) {
            let text = line.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { continue }
            if text.count == 3, let value = Int(text), (100...599).contains(value) { code = value }
            else if !text.allSatisfy({ $0.isNumber }) { diagnostics.append(text) }
        }
        let transport = result.code == 0 ? nil : (diagnostics.last ?? "curl 退出码 \(result.code)")
        return InternetPolicy.classify(code: code, elapsed: Int(Date().timeIntervalSince(start) * 1000), transportError: transport)
    }
    func probe(port: Int) async throws -> ProbeResult {
        let start = Date()
        // curl uses a dedicated HTTP proxy listener, TLS verification stays enabled, no redirects.
        let r = try await Command.run("/usr/bin/curl", ["--silent", "--show-error", "--noproxy", "", "--proxy", "http://127.0.0.1:\(port)",
            "--connect-timeout", "4", "--max-time", "8", "--max-filesize", "131072", "--write-out", "\n%{http_code}",
            "--header", "Cache-Control: no-cache", "https://api.openai.com/v1/models"], cancellable: true)
        let ms = Int(Date().timeIntervalSince(start) * 1000)
        guard r.code == 0, let cut = r.text.lastIndex(of: "\n"), let code = Int(r.text[r.text.index(after: cut)...]) else {
            return ProbeResult(health: .offline, milliseconds: ms, detail: "连接失败 / TLS 错误 / 超时")
        }
        return HealthPolicy.classify(code: code, body: String(r.text[..<cut]), elapsed: ms)
    }
    func throughput(port: Int) async throws -> ThroughputSample? {
        let result = try await Command.run("/usr/bin/curl", ["--silent", "--show-error", "--noproxy", "", "--proxy", "http://127.0.0.1:\(port)",
            "--connect-timeout", "3", "--max-time", "10", "--max-filesize", "1048576", "--output", "/dev/null",
            "--header", "Accept-Encoding: identity", "--write-out", "%{speed_download}\n%{size_download}\n%{http_code}",
            "https://speed.cloudflare.com/__down?bytes=1048576"], cancellable: true)
        let fields = result.text.components(separatedBy: .newlines)
        guard result.code == 0, fields.count == 3, fields[2] == "200", let speed = Double(fields[0]), speed > 0,
              let bytes = Double(fields[1]), bytes >= 1_048_576 else { return nil }
        return ThroughputSample(bytesPerSecond: speed, downloadedBytes: Int(bytes))
    }
    func trafficStream() -> AsyncThrowingStream<TrafficSample, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    var request = URLRequest(url: URL(string: "http://127.0.0.1:\(Configuration.controllerPort)/traffic")!)
                    request.timeoutInterval = 86_400
                    request.setValue("Bearer \(secret)", forHTTPHeaderField: "Authorization")
                    let (bytes, response) = try await session.bytes(for: request)
                    guard let response = response as? HTTPURLResponse, response.statusCode == 200 else {
                        throw ProxyError.message("无法读取内核流量统计")
                    }
                    for try await line in bytes.lines {
                        try Task.checkCancellation()
                        if let sample = try? JSONDecoder().decode(TrafficSample.self, from: Data(line.utf8)) { continuation.yield(sample) }
                    }
                    continuation.finish()
                } catch { continuation.finish(throwing: error) }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
    func connections() async throws -> ConnectionSnapshot {
        let document = try await request("/connections")
        return try JSONDecoder().decode(ConnectionSnapshot.self, from: JSONSerialization.data(withJSONObject: document))
    }
}
