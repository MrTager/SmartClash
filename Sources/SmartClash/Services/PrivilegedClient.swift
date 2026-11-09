import Foundation
import ServiceManagement
import PrivilegedSupport
import ProxyLogic
import Darwin
import LifecycleSupport

private final class XPCReply<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Value, Error>?
    private var connection: NSXPCConnection?
    private var timeout: DispatchWorkItem?
    private let invalidatesConnection: Bool
    init(_ continuation: CheckedContinuation<Value, Error>, connection: NSXPCConnection, seconds: TimeInterval,
         invalidatesConnection: Bool = true) {
        self.continuation = continuation; self.connection = connection
        self.invalidatesConnection = invalidatesConnection
        let timeout = DispatchWorkItem { [weak self] in
            self?.finish(.failure(ProxyError.message("后台服务响应超时，请恢复网络后重试")))
        }
        self.timeout = timeout
        DispatchQueue.global().asyncAfter(deadline: .now() + seconds, execute: timeout)
    }
    func finish(_ result: Result<Value, Error>) {
        lock.lock()
        let waiting = continuation; continuation = nil
        let connection = self.connection; self.connection = nil
        let timeout = self.timeout; self.timeout = nil
        lock.unlock()
        timeout?.cancel(); waiting?.resume(with: result)
        if invalidatesConnection { connection?.invalidate() }
    }
}

@MainActor
final class PrivilegedClient {
    static let shared = PrivilegedClient()
    private var installation: HelperInstallation?
    private var preparation: Task<Void, Error>?
    private var sessionConnection: NSXPCConnection?
    private let nonce = UUID().uuidString
    private var sessionPreparation: Task<Void, Error>?
    private var terminating = false
    private var sessionLost = false
    private var sessionReplies: [UUID: (NSXPCConnection, (Error) -> Void)] = [:]
    private(set) var proxyPending = false
    private(set) var tunPending = false
    var hasSession: Bool { sessionConnection != nil }
    func closeGate() { terminating = true }
    func reconcileAtStartup() async throws {
        try checkGate()
        guard FileManager.default.fileExists(atPath: HelperLocation.manifest) else {
            guard !FileManager.default.fileExists(atPath: HelperLocation.corePlist) else { throw LifecycleError.refused("发现旧 TUN 服务但没有可信安装记录") }
            return
        }
        // 不在启动恢复时安装或提示授权；无法联络已安装 helper 时禁止业务初始化。
        let installed = try PropertyListDecoder().decode(HelperInstallation.self, from: Data(contentsOf: URL(fileURLWithPath: HelperLocation.manifest)))
        guard installed.version == HelperLocation.version, installed.ownerUID == getuid() else { throw LifecycleError.refused("后台服务需要更新；启动对账未完成") }
        installation = installed
        try await openSession()
    }
    private func checkGate() throws {
        guard !terminating else { throw CancellationError() }
        try Task.checkCancellation()
    }
    private func openSession(cleanup: Bool = false) async throws {
        if !cleanup { try checkGate() }
        if sessionConnection != nil { return }
        guard cleanup || !sessionLost else { throw LifecycleError.refused("后台会话已断开，恢复未确认；请停止并重试退出") }
        if let sessionPreparation { return try await sessionPreparation.value }
        let task = Task { @MainActor in
            if !cleanup { try self.checkGate() }
            let connection = NSXPCConnection(machServiceName: HelperLocation.label, options: .privileged)
            connection.setCodeSigningRequirement(try self.identity().helperRequirement)
            connection.remoteObjectInterface = NSXPCInterface(with: PrivilegedXPC.self)
            connection.invalidationHandler = { [weak self, weak connection] in Task { @MainActor in
                guard let self, let connection else { return }
                self.connectionLost(connection)
            } }
            connection.interruptionHandler = { connection.invalidate() }
            connection.resume()
            do {
                try await self.sessionOperation(connection) { $0.beginSession(self.nonce, withReply: $1) }
                // 即使退出已开始也先保留连接，交给 endSession 确认恢复后关闭。
                self.sessionConnection = connection
                self.sessionLost = false
            } catch { self.sessionLost = true; connection.invalidate(); throw error }
        }
        sessionPreparation = task
        defer { sessionPreparation = nil }
        try await task.value
        if !cleanup { try checkGate() }
    }
    private func connectionLost(_ connection: NSXPCConnection) {
        if sessionConnection === connection { sessionConnection = nil; sessionLost = true }
        let waiting = sessionReplies.filter { $0.value.0 === connection }
        for (id, entry) in waiting {
            sessionReplies[id] = nil
            entry.1(LifecycleError.refused("后台会话连接已断开，恢复未确认"))
        }
    }
    func endSession() async throws {
        terminating = true
        if let preparation { _ = try? await preparation.value }
        if let sessionPreparation { _ = try? await sessionPreparation.value }
        // 仅为确认清理重连，不安装、不授权、不重新开放业务 gate。
        if sessionConnection == nil && sessionLost { try await openSession(cleanup: true) }
        guard let connection = sessionConnection else {
            guard !sessionLost && !proxyPending && !tunPending else { throw LifecycleError.refused("后台会话失联，恢复未确认") }
            return
        }
        do { try await sessionOperation(connection) { $0.endSession(self.nonce, withReply: $1) } }
        catch {
            // 超时也可能是已清理但回包丢失；下次仅重连确认，不沿用已结束的 endpoint。
            sessionConnection = nil; sessionLost = true; connection.invalidate()
            throw error
        }
        proxyPending = false; tunPending = false; sessionLost = false
        sessionConnection = nil; connection.invalidate()
    }
    private var helperURL: URL { Bundle.main.bundleURL.appendingPathComponent("Contents/Library/HelperTools/SmartClashHelper") }
    private var resources: URL { Bundle.main.bundleURL.appendingPathComponent("Contents/Resources") }
    private var coreURLs: [(String, URL)] {
        ["mihomo", "sing-box"].map { ($0, resources.appendingPathComponent($0)) }
            .filter { FileManager.default.isExecutableFile(atPath: $0.1.path) }
    }

    func prepare() async throws {
        try checkGate()
        if let preparation { try await preparation.value; try checkGate(); return }
        let task = Task {
            try self.checkGate()
            try await self.ensureAvailable()
            try self.checkGate()
            try await self.openSession()
        }
        preparation = task
        defer { preparation = nil }
        try await task.value
        try checkGate()
    }
    func setSystemProxy(enabled: Bool) async throws {
        if enabled { try await prepare(); try checkGate() }
        guard let connection = sessionConnection else {
            if !enabled && !proxyPending && !sessionLost { return }
            throw LifecycleError.refused("后台会话不可用，代理恢复未确认")
        }
        if enabled { proxyPending = true }
        try await sessionOperation(connection) { $0.setSystemProxy(self.nonce, port: Configuration.proxyPort, enabled: enabled, withReply: $1) }
        if !enabled { proxyPending = false }
        else { try checkGate() }
    }
    func startTun(_ configuration: Data, kernel: KernelKind) async throws {
        try checkGate()
        try TunConfiguration.validate(configuration, kernel: kernel, port: Configuration.proxyPort)
        try await prepare()
        try checkGate()
        guard let connection = sessionConnection else { throw LifecycleError.refused("后台会话不可用") }
        tunPending = true
        try await sessionOperation(connection) {
            $0.startTun(configuration, kernel: kernel.rawValue, port: Configuration.proxyPort, session: self.nonce, withReply: $1)
        }
        try checkGate()
    }
    func stopTun() async throws {
        guard let connection = sessionConnection else {
            guard !tunPending && !sessionLost else { throw LifecycleError.refused("后台会话失联，TUN 恢复未确认") }
            return
        }
        try await sessionOperation(connection) { $0.stopTun(self.nonce, withReply: $1) }
        tunPending = false
    }
    private func identity() throws -> HelperInstallation {
        if let installation { return installation }
        var requirements: [String: String] = [:]
        for (name, url) in coreURLs {
            requirements[name] = try CodeIdentity.requirement(at: url)
        }
        guard let core = requirements["mihomo"] ?? requirements["sing-box"] else {
            throw ProxyError.message("安装包缺少代理内核，请先运行 script/fetch_core.sh 并重新构建")
        }
        let value = try HelperInstallation(build: Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "development",
            ownerUID: getuid(), clientRequirement: CodeIdentity.requirement(at: Bundle.main.bundleURL),
            helperRequirement: CodeIdentity.requirement(at: helperURL), coreRequirement: core,
            kernelRequirements: requirements)
        installation = value; return value
    }
    private func ensureAvailable() async throws {
        let expected = try identity()
        let installed = try? PropertyListDecoder().decode(HelperInstallation.self, from: Data(contentsOf: URL(fileURLWithPath: HelperLocation.manifest)))
        if installed == expected && FileManager.default.fileExists(atPath: HelperLocation.plist) {
            do {
                let version: Int = try await request(seconds: 8) { service, finish in service.ping { finish(.success($0)) } }
                guard version == HelperLocation.version else { throw ProxyError.message("后台服务版本不匹配，请重新安装应用") }
                return
            } catch {
                if SMAppService.statusForLegacyPlist(at: URL(fileURLWithPath: HelperLocation.plist)) == .requiresApproval {
                    SMAppService.openSystemSettingsLoginItems()
                    throw ProxyError.message("请在系统设置的“登录项与扩展”中允许 SmartClash 后台服务，之后开关无需重复授权")
                }
                // A temporarily unavailable or user-disabled service must not cause another password prompt.
                throw ProxyError.message("SmartClash 后台服务暂不可用。请检查系统设置中的后台运行权限，或重启电脑后重试")
            }
        }
        try checkGate()
        try await install(expected)
        try checkGate()
        var lastError: Error = ProxyError.message("后台服务安装后未能启动")
        for _ in 0..<10 {
            do {
                let version: Int = try await request(seconds: 3) { service, finish in service.ping { finish(.success($0)) } }
                guard version == HelperLocation.version else { throw ProxyError.message("后台服务版本不匹配") }
                return
            } catch { lastError = error }
            try await Task.sleep(nanoseconds: 250_000_000)
        }
        throw lastError
    }
    private func install(_ installation: HelperInstallation) async throws {
        let manifest = try PropertyListEncoder().encode(installation).base64EncodedString()
        let plist: [String: Any] = ["Label": HelperLocation.label,
            "ProgramArguments": [HelperLocation.executable],
            "MachServices": [HelperLocation.label: true], "KeepAlive": false,
            "ProcessType": "Interactive", "Umask": 0o077]
        let stagedPlist = try PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0).base64EncodedString()
        let q = Command.quote
        var kernelInstalls: [String] = []
        var kernelVerifies: [String] = []
        var kernelMoves: [String] = []
        for (name, url) in coreURLs {
            let staged = HelperLocation.directory + "/" + name + ".new"
            kernelInstalls.append("/usr/bin/install -o root -g wheel -m 755 \(q(url.path)) \(q(staged))")
            if let requirement = installation.kernelRequirements?[name] {
                kernelVerifies.append("/usr/bin/codesign --verify --strict -R \(q(CodeIdentity.commandLineRequirement(requirement))) \(q(staged))")
            }
            kernelMoves.append("/bin/mv -f \(q(staged)) \(q(HelperLocation.directory + "/" + name))")
        }
        guard !kernelInstalls.isEmpty else { throw ProxyError.message("安装包缺少代理内核，请先运行 script/fetch_core.sh") }
        // Administrator consent is used only to install/update root-owned, signed code.
        // Subsequent operations use a small, authenticated XPC interface; no shell is exposed over XPC.
        let script = """
        set -e
        umask 077
        /usr/bin/install -d -o root -g wheel -m 755 /Library/PrivilegedHelperTools \(q(HelperLocation.directory))
        /usr/bin/install -o root -g wheel -m 755 \(q(helperURL.path)) \(q(HelperLocation.executable + ".new"))
        \(kernelInstalls.joined(separator: "\n"))
        /usr/bin/codesign --verify --strict -R \(q(CodeIdentity.commandLineRequirement(installation.helperRequirement))) \(q(HelperLocation.executable + ".new"))
        \(kernelVerifies.joined(separator: "\n"))
        if /bin/launchctl print system/\(HelperLocation.label) >/dev/null 2>&1; then /bin/launchctl bootout system/\(HelperLocation.label); fi
        /bin/mv -f \(q(HelperLocation.executable + ".new")) \(q(HelperLocation.executable))
        \(kernelMoves.joined(separator: "\n"))
        /usr/bin/printf '%s' \(q(manifest)) | /usr/bin/base64 -D > \(q(HelperLocation.manifest + ".new"))
        /usr/bin/printf '%s' \(q(stagedPlist)) | /usr/bin/base64 -D > \(q(HelperLocation.plist + ".new"))
        /usr/sbin/chown root:wheel \(q(HelperLocation.manifest + ".new")) \(q(HelperLocation.plist + ".new"))
        /bin/chmod 644 \(q(HelperLocation.manifest + ".new")) \(q(HelperLocation.plist + ".new"))
        /bin/mv -f \(q(HelperLocation.manifest + ".new")) \(q(HelperLocation.manifest))
        /bin/mv -f \(q(HelperLocation.plist + ".new")) \(q(HelperLocation.plist))
        /bin/launchctl bootstrap system \(q(HelperLocation.plist))
        """
        try await Command.admin(script)
    }
    private func sessionOperation(_ connection: NSXPCConnection,
                                  send: (PrivilegedXPC, @escaping (String?) -> Void) -> Void) async throws {
        let id = UUID()
        defer { sessionReplies[id] = nil }
        let _: Void = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            let reply = XPCReply(continuation, connection: connection, seconds: 120, invalidatesConnection: false)
            sessionReplies[id] = (connection, { reply.finish(.failure($0)) })
            guard let proxy = connection.remoteObjectProxyWithErrorHandler({ reply.finish(.failure($0)) }) as? PrivilegedXPC else {
                reply.finish(.failure(ProxyError.message("无法连接后台会话"))); return
            }
            send(proxy) { error in
                if let error { reply.finish(.failure(ProxyError.message(error))) }
                else { reply.finish(.success(())) }
            }
        }
    }
    private func request<Value>(seconds: TimeInterval, send: (PrivilegedXPC, @escaping (Result<Value, Error>) -> Void) -> Void) async throws -> Value {
        let requirement = try identity().helperRequirement
        return try await withCheckedThrowingContinuation { continuation in
            let connection = NSXPCConnection(machServiceName: HelperLocation.label, options: .privileged)
            connection.setCodeSigningRequirement(requirement)
            connection.remoteObjectInterface = NSXPCInterface(with: PrivilegedXPC.self)
            let reply = XPCReply(continuation, connection: connection, seconds: seconds)
            connection.invalidationHandler = { reply.finish(.failure(ProxyError.message("后台服务连接已断开"))) }
            connection.interruptionHandler = { reply.finish(.failure(ProxyError.message("后台服务连接中断，请恢复网络后重试"))) }
            connection.resume()
            guard let proxy = connection.remoteObjectProxyWithErrorHandler({ reply.finish(.failure($0)) }) as? PrivilegedXPC else {
                reply.finish(.failure(ProxyError.message("无法连接后台服务"))); return
            }
            send(proxy) { reply.finish($0) }
        }
    }
}
