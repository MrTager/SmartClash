import Foundation
import ProxyLogic
@MainActor
final class CoreProcess {
    private var process: Process?
    private var logHandle: FileHandle?
    private(set) var privileged = false
    private let pidFile = Files.root.appendingPathComponent("core.pid")
    private var rootServicePresent: Bool { FileManager.default.fileExists(atPath: Self.plist) }
    static let label = "com.smartclash.core"
    static let plist = "/Library/LaunchDaemons/com.smartclash.core.plist"
    static let install = "/Library/Application Support/SmartClashCore"
    var recoveryNeeded: Bool { rootServicePresent || FileManager.default.fileExists(atPath: pidFile.path) }
    var executable: URL { Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/mihomo") }

    func start(data: Data, tun: Bool, api: CoreClient) async throws {
        guard process == nil, !recoveryNeeded else { throw ProxyError.message("请先停止内核 / 恢复遗留服务") }
        guard FileManager.default.isExecutableFile(atPath: executable.path) else { throw ProxyError.message("安装包缺少 Mihomo 内核，请先运行 script/fetch_core.sh 并重新构建") }
        // Refuse to attach to another application's controller or occupied proxy ports.
        for port in [Configuration.controllerPort, Configuration.proxyPort, Configuration.probePort] {
            let r = try await Command.run("/usr/sbin/lsof", ["-nP", "-iTCP:\(port)", "-sTCP:LISTEN", "-t"])
            guard r.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw ProxyError.message("端口 \(port) 已占用，请关闭冲突的代理程序") }
        }
        let config = Files.root.appendingPathComponent("config.json")
        try Files.write(data, to: config)
        let test = try await Command.run(executable.path, ["-t", "-d", Files.root.path, "-f", config.path])
        try Files.write(Data(test.text.utf8), to: Files.root.appendingPathComponent("core-test.log"))
        guard test.code == 0 else { throw ProxyError.message("内核配置校验失败，请检查订阅节点格式（详情见 core-test.log）") }
        if tun {
            let root = Self.install
            let plist: [String: Any] = ["Label": Self.label,
                "ProgramArguments": [root + "/mihomo", "-d", root, "-f", root + "/config.json"],
                "RunAtLoad": true, "KeepAlive": false,
                "StandardOutPath": root + "/core.log", "StandardErrorPath": root + "/core.log",
                "WorkingDirectory": root, "Umask": 0o077]
            let temp = Files.root.appendingPathComponent("core.plist")
            try Files.write(PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0), to: temp)
            let q = Command.quote
            try await Command.admin("set -e\n/usr/bin/install -d -o root -g wheel -m 700 \(q(root))\n/usr/bin/install -o root -g wheel -m 755 \(q(executable.path)) \(q(root + "/mihomo"))\n/usr/bin/install -o root -g wheel -m 600 \(q(config.path)) \(q(root + "/config.json"))\n/usr/bin/install -o root -g wheel -m 600 \(q(temp.path)) \(q(Self.plist))\n/bin/launchctl bootstrap system \(q(Self.plist))")
            privileged = true
        } else {
            let log = Files.root.appendingPathComponent("core.log")
            try Files.write(Data(), to: log)
            logHandle = try FileHandle(forWritingTo: log)
            let child = Process(); child.executableURL = executable
            child.arguments = ["-d", Files.root.path, "-f", config.path]
            child.standardOutput = logHandle; child.standardError = logHandle
            try child.run(); process = child
            do { try Files.write(Data(String(child.processIdentifier).utf8), to: pidFile) }
            catch { child.terminate(); throw error }
        }
        do {
            for _ in 0..<40 {
                if let p = process, !p.isRunning { throw ProxyError.message("内核提前退出，请查看 core.log") }
                if (try? await api.request("/version")) != nil { return }
                try await Task.sleep(nanoseconds: 250_000_000)
            }
            throw ProxyError.message("内核启动超时，请检查 core.log")
        } catch { try? await stop(); throw error }
    }
    func stop() async throws {
        if privileged || rootServicePresent {
            let q = Command.quote
            // The fixed launchd label avoids killing unrelated proxy processes.
            try await Command.admin("set -e\nif /bin/launchctl print system/\(Self.label) >/dev/null 2>&1; then /bin/launchctl bootout system/\(Self.label); fi\n/bin/rm -f \(q(Self.plist))\n/bin/rm -rf \(q(Self.install))")
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
            let text = try String(contentsOf: pidFile, encoding: .utf8)
            guard let pid = Int32(text), pid > 1 else { throw ProxyError.message("内核 PID 恢复记录无效") }
            let identity = try await Command.run("/usr/sbin/lsof", ["-a", "-p", String(pid), "-d", "txt", "-Fn"])
            if identity.text.components(separatedBy: .newlines).contains("n" + executable.path) {
                _ = try await Command.run("/bin/kill", ["-TERM", String(pid)])
                for _ in 0..<30 {
                    let alive = try await Command.run("/bin/kill", ["-0", String(pid)])
                    if alive.code != 0 { break }
                    await Task.detached { try? await Task.sleep(nanoseconds: 100_000_000) }.value
                }
                let alive = try await Command.run("/bin/kill", ["-0", String(pid)])
                guard alive.code != 0 else { throw ProxyError.message("遗留内核尚未退出，请重试恢复网络") }
            }
            try FileManager.default.removeItem(at: pidFile)
        }
        try? logHandle?.close(); logHandle = nil
    }
}
