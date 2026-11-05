import Foundation
import ProxyLogic
import PrivilegedSupport
@MainActor
final class CoreProcess {
    private var process: Process?
    private var logHandle: FileHandle?
    private(set) var privileged = false
    private(set) var kind: KernelKind = .mihomo
    private let pidFile = Files.root.appendingPathComponent("core.pid")
    private var rootServicePresent: Bool { FileManager.default.fileExists(atPath: Self.plist) }
    static let label = HelperLocation.coreLabel
    static let plist = HelperLocation.corePlist
    static let install = HelperLocation.coreDirectory
    var recoveryNeeded: Bool { rootServicePresent || FileManager.default.fileExists(atPath: pidFile.path) }
    var executable: URL { resources.appendingPathComponent(kind.binaryName) }
    var assetDirectory: String { resources.path }
    var alive: Bool {
        if privileged { return rootServicePresent }
        return process?.isRunning == true
    }
    private var resources: URL { Bundle.main.bundleURL.appendingPathComponent("Contents/Resources") }

    func configure(kind: KernelKind) { self.kind = kind }

    func start(data: Data, tun: Bool, api: CoreClient, kind: KernelKind) async throws {
        self.kind = kind
        try Task.checkCancellation()
        guard process == nil, !recoveryNeeded else { throw ProxyError.message("请先停止内核 / 恢复遗留服务") }
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw ProxyError.message("安装包缺少 \(kind.title) 内核，请先运行 script/fetch_core.sh 并重新构建")
        }
        // Refuse to attach to another application's controller or occupied proxy ports.
        let ports = kind == .xray
            ? [Configuration.proxyPort, KernelConfiguration.xraySocksPort, KernelConfiguration.xrayAPIPort]
            : [Configuration.controllerPort, Configuration.proxyPort, Configuration.probePort]
        for port in ports {
            let r = try await Command.run("/usr/sbin/lsof", ["-nP", "-iTCP:\(port)", "-sTCP:LISTEN", "-t"], cancellable: true)
            guard r.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw ProxyError.message("端口 \(port) 已占用，请关闭冲突的代理程序")
            }
        }
        let config = Files.root.appendingPathComponent("config.json")
        try Files.write(data, to: config)
        try await validate(data: data)
        try Task.checkCancellation()
        if tun {
            guard kind.capabilities.tun else { throw ProxyError.message("\(kind.title) 内核不支持 TUN，请改用 Mihomo 或 sing-box") }
            try await PrivilegedClient.shared.startTun(data, kernel: kind)
            privileged = true
        } else {
            let log = Files.root.appendingPathComponent("core.log")
            try Files.write(Data(), to: log)
            logHandle = try FileHandle(forWritingTo: log)
            let child = Process(); child.executableURL = executable
            child.arguments = arguments(config: config.path)
            child.environment = environment
            child.standardOutput = logHandle; child.standardError = logHandle
            try child.run(); process = child
            do { try Files.write(Data("{\"pid\":\(child.processIdentifier),\"kernel\":\"\(kind.rawValue)\"}".utf8), to: pidFile) }
            catch { child.terminate(); throw error }
        }
        do {
            for _ in 0..<40 {
                try Task.checkCancellation()
                if let p = process, !p.isRunning { throw ProxyError.message("内核提前退出，请查看 core.log") }
                if try await ready(api: api) { return }
                try await Task.sleep(nanoseconds: 250_000_000)
            }
            throw ProxyError.message("内核启动超时，请检查 core.log")
        } catch { try? await stop(); throw error }
    }

    /// Run the kernel's own configuration test without touching the running core.
    func validate(data: Data) async throws {
        let temp = Files.root.appendingPathComponent("validate.json")
        try Files.write(data, to: temp)
        let test = try await Command.run(executable.path, testArguments(config: temp.path), cancellable: true, environment: environment)
        try Files.write(Data(test.text.utf8), to: Files.root.appendingPathComponent("core-test.log"))
        guard test.code == 0 else { throw ProxyError.message("内核配置校验失败，请检查订阅节点格式（详情见 core-test.log）") }
    }

    func stop() async throws {
        if privileged || rootServicePresent {
            // The fixed launchd label avoids killing unrelated proxy processes.
            try await PrivilegedClient.shared.stopTun()
            privileged = false
        }
        if let p = process {
            if p.isRunning {
                p.terminate()
                for _ in 0..<30 { if !p.isRunning { break }; await Task.detached { try? await Task.sleep(nanoseconds: 100_000_000) }.value }
                if p.isRunning { throw ProxyError.message("内核尚未退出，请稍后重试停止") }
            }
            process = nil
        }
        // An ordinary core can survive an abrupt GUI crash; validate its executable before termination.
        if FileManager.default.fileExists(atPath: pidFile.path) {
            let record = try readRecoveryRecord()
            let identity = try await Command.run("/usr/sbin/lsof", ["-a", "-p", String(record.pid), "-d", "txt", "-Fn"])
            let expected = resources.appendingPathComponent(record.kind.binaryName).path
            if identity.text.components(separatedBy: .newlines).contains("n" + expected) {
                _ = try await Command.run("/bin/kill", ["-TERM", String(record.pid)])
                for _ in 0..<30 {
                    let alive = try await Command.run("/bin/kill", ["-0", String(record.pid)])
                    if alive.code != 0 { break }
                    await Task.detached { try? await Task.sleep(nanoseconds: 100_000_000) }.value
                }
                let alive = try await Command.run("/bin/kill", ["-0", String(record.pid)])
                guard alive.code != 0 else { throw ProxyError.message("遗留内核尚未退出，请重试恢复网络") }
            }
            try FileManager.default.removeItem(at: pidFile)
        }
        try? logHandle?.close(); logHandle = nil
    }

    private var environment: [String: String] {
        kind == .xray ? ["XRAY_LOCATION_ASSET": assetDirectory] : [:]
    }

    private func arguments(config: String) -> [String] {
        switch kind {
        case .mihomo: return ["-d", Files.root.path, "-f", config]
        case .singBox: return ["run", "-c", config, "-D", Files.root.path]
        case .xray: return ["run", "-c", config]
        }
    }

    private func testArguments(config: String) -> [String] {
        switch kind {
        case .mihomo: return ["-t", "-d", Files.root.path, "-f", config]
        case .singBox: return ["check", "-c", config, "-D", Files.root.path]
        case .xray: return ["run", "-test", "-c", config]
        }
    }

    private func ready(api: CoreClient) async throws -> Bool {
        switch kind {
        case .mihomo, .singBox:
            return (try? await api.request("/version")) != nil
        case .xray:
            let r = try await Command.run("/usr/sbin/lsof", ["-nP", "-iTCP:\(Configuration.proxyPort)", "-sTCP:LISTEN", "-t"])
            return !r.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    private func readRecoveryRecord() throws -> (pid: Int32, kind: KernelKind) {
        let text = try String(contentsOf: pidFile, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasPrefix("{"),
           let data = text.data(using: .utf8),
           let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let raw = object["pid"] as? Int, raw > 1 {
            return (Int32(raw), KernelKind(rawValue: object["kernel"] as? String ?? "") ?? .mihomo)
        }
        // Version 0.8 wrote a bare PID and always ran Mihomo.
        guard let pid = Int32(text), pid > 1 else { throw ProxyError.message("内核 PID 恢复记录无效") }
        return (pid, .mihomo)
    }
}
