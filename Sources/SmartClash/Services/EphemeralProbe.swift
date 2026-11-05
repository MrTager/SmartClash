import Foundation
import Darwin
import ProxyLogic

/// 临时单节点内核探测。
///
/// Xray 没有 Clash 控制接口，无法通过 PROBE 组测速；这里按需启动一个只监听
/// 本机随机端口的临时内核，操作完成后立即结束。Mihomo / sing-box 也可以复用。
enum EphemeralProbe {
    static func run<T>(kind: KernelKind, node: ProxyNode,
                       operation: (Int) async throws -> T) async throws -> T {
        let resources = Bundle.main.bundleURL.appendingPathComponent("Contents/Resources")
        let executable = resources.appendingPathComponent(kind.binaryName)
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw ProxyError.message("安装包缺少 \(kind.title) 内核，请先运行 script/fetch_core.sh 并重新构建")
        }
        guard let port = await freePort() else { throw ProxyError.message("无法分配临时探测端口") }
        let directory = Files.root.appendingPathComponent("probes").appendingPathComponent("probe-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: 0o700])
        let config = directory.appendingPathComponent("probe.json")
        try Files.write(try KernelConfiguration.probeConfig(kind: kind, node: node, port: port), to: config)
        let log = directory.appendingPathComponent("probe.log")
        FileManager.default.createFile(atPath: log.path, contents: nil, attributes: [.posixPermissions: 0o600])
        let handle = try FileHandle(forWritingTo: log)

        let process = Process()
        process.executableURL = executable
        process.currentDirectoryURL = directory
        process.standardOutput = handle
        process.standardError = handle
        switch kind {
        case .mihomo:
            process.arguments = ["-d", directory.path, "-f", config.path]
        case .singBox:
            process.arguments = ["run", "-c", config.path, "-D", directory.path]
        case .xray:
            process.arguments = ["run", "-c", config.path]
            process.environment = ["XRAY_LOCATION_ASSET": resources.path]
        }
        try process.run()
        defer {
            if process.isRunning { process.terminate() }
            for _ in 0..<30 where process.isRunning { usleep(50_000) }
            try? handle.close()
            if process.isRunning { kill(process.processIdentifier, SIGKILL) }
            // 仅清理本次探测创建的临时目录。
            try? FileManager.default.removeItem(at: directory)
        }
        try await waitReady(process: process, port: port, log: log)
        return try await operation(port)
    }

    static func latency(kind: KernelKind, node: ProxyNode, timeout: Int = 5000) async -> NodeLatency {
        let started = Date()
        do {
            let result = try await run(kind: kind, node: node) { port in
                try await CoreClient(secret: "").internetProbe(port: port)
            }
            guard result.health == .healthy else {
                return NodeLatency(milliseconds: nil, failure: result.detail)
            }
            let elapsed = Int(Date().timeIntervalSince(started) * 1000)
            return NodeLatency(milliseconds: max(result.milliseconds, elapsed))
        } catch is CancellationError {
            return NodeLatency(milliseconds: nil, failure: "已取消")
        } catch {
            return NodeLatency(milliseconds: nil, failure: error.localizedDescription)
        }
    }

    private static func waitReady(process: Process, port: Int, log: URL) async throws {
        for _ in 0..<80 {
            try Task.checkCancellation()
            guard process.isRunning else {
                throw ProxyError.message("探测内核提前退出\(logTail(log))")
            }
            if let result = try? await Command.run("/usr/sbin/lsof",
                ["-nP", "-iTCP:\(port)", "-sTCP:LISTEN", "-t"]),
               !result.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                return
            }
            try await Task.sleep(nanoseconds: 120_000_000)
        }
        throw ProxyError.message("探测内核启动超时\(logTail(log))")
    }

    private static func logTail(_ url: URL) -> String {
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return "" }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        return "：" + String(trimmed.suffix(240))
    }

    private static func freePort() async -> Int? {
        for _ in 0..<20 {
            let port = Int.random(in: 20_000...60_000)
            if let result = try? await Command.run("/usr/sbin/lsof",
                ["-nP", "-iTCP:\(port)", "-sTCP:LISTEN", "-t"]),
               result.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                return port
            }
        }
        return nil
    }
}
