import Foundation
import ProxyLogic
import PrivilegedSupport

@MainActor
final class CoreProcess {
    private var childID: String?
    private var terminating = false
    private(set) var privileged = false
    private(set) var kind: KernelKind = .mihomo
    private let pidFile = Files.root.appendingPathComponent("core.pid")
    private var rootServicePresent: Bool { FileManager.default.fileExists(atPath: Self.plist) }
    static let label = HelperLocation.coreLabel
    static let plist = HelperLocation.corePlist
    static let install = HelperLocation.coreDirectory
    var recoveryNeeded: Bool {
        childID != nil || privileged || PrivilegedClient.shared.tunPending || rootServicePresent || FileManager.default.fileExists(atPath: pidFile.path)
    }
    var executable: URL { resources.appendingPathComponent(kind.binaryName) }
    var assetDirectory: String { resources.path }
    var alive: Bool { privileged ? rootServicePresent : childID != nil }
    private var resources: URL { Bundle.main.bundleURL.appendingPathComponent("Contents/Resources") }

    func configure(kind: KernelKind) { self.kind = kind }
    func closeGate() { terminating = true }
    func confirmGuardianEnded() { childID = nil }
    func confirmPrivilegedSessionEnded() { privileged = false }
    private func checkGate() throws {
        guard !terminating else { throw CancellationError() }
        try Task.checkCancellation()
    }

    func start(data: Data, tun: Bool, api: CoreClient, kind: KernelKind) async throws {
        try checkGate()
        self.kind = kind
        guard !recoveryNeeded else { throw ProxyError.message("请先停止内核 / 恢复遗留服务") }
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw ProxyError.message("安装包缺少 \(kind.title) 内核，请先运行 script/fetch_core.sh 并重新构建")
        }
        let ports = kind == .xray
            ? [Configuration.proxyPort, KernelConfiguration.xraySocksPort, KernelConfiguration.xrayAPIPort]
            : [Configuration.controllerPort, Configuration.proxyPort, Configuration.probePort]
        for port in ports {
            let r = try await Command.run("/usr/sbin/lsof", ["-nP", "-iTCP:\(port)", "-sTCP:LISTEN", "-t"], cancellable: true)
            try checkGate()
            guard !recoveryNeeded else { throw ProxyError.message("内核启动请求已过期，请先停止内核") }
            guard r.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw ProxyError.message("端口 \(port) 已占用，请关闭冲突的代理程序")
            }
        }
        let config = Files.root.appendingPathComponent("config.json")
        try Files.write(data, to: config)
        try await validate(data: data)
        try checkGate()
        do {
            if tun {
                guard kind.capabilities.tun else { throw ProxyError.message("\(kind.title) 内核不支持 TUN，请改用 Mihomo 或 sing-box") }
                try await PrivilegedClient.shared.startTun(data, kernel: kind)
                privileged = true
            } else {
                childID = try await GuardianClient.shared.spawn(executable: executable, arguments: arguments(config: config.path),
                                                               directory: Files.root, environment: environment)
            }
            for _ in 0..<40 {
                try checkGate()
                if let id = childID {
                    let status = try await GuardianClient.shared.status(id)
                    try checkGate()
                    guard status.running == true else { throw ProxyError.message("内核提前退出，请查看 guardian-\(id).log") }
                }
                if try await ready(api: api) { try checkGate(); return }
                try await Task.sleep(nanoseconds: 250_000_000)
            }
            throw ProxyError.message("内核启动超时，请检查 guardian 日志或 TUN 的 core.log")
        } catch { try? await stop(); throw error }
    }

    func validate(data: Data) async throws {
        try checkGate()
        let temp = Files.root.appendingPathComponent("validate-\(UUID().uuidString).json")
        try Files.write(data, to: temp)
        defer { try? FileManager.default.removeItem(at: temp) }
        let code = try await GuardianClient.shared.validate(executable: executable, arguments: testArguments(config: temp.path),
                                                            directory: Files.root, environment: environment)
        try checkGate()
        guard code == 0 else { throw ProxyError.message("内核配置校验失败，请检查订阅节点格式（详情见 guardian-*.log）") }
    }

    func stop() async throws {
        var failures: [String] = []
        if privileged || PrivilegedClient.shared.tunPending || rootServicePresent {
            do {
                try await PrivilegedClient.shared.stopTun()
                guard !rootServicePresent else { throw ProxyError.message("TUN 服务恢复未确认") }
                privileged = false
            } catch { failures.append(error.localizedDescription) }
        }
        if let id = childID {
            do { try await GuardianClient.shared.stop(id); childID = nil }
            catch { failures.append(error.localizedDescription) }
        }
        // 旧版 PID 文件不包含可靠启动身份，不再按 PID / lsof 猜测归属并发信号。
        if FileManager.default.fileExists(atPath: pidFile.path) {
            failures.append("发现旧版 core.pid，无法验证进程启动身份；记录已保留，请核实遗留内核")
        }
        if !failures.isEmpty { throw ProxyError.message(failures.joined(separator: "；")) }
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
}
