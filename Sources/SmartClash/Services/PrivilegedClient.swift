import Foundation
import ServiceManagement
import PrivilegedSupport
import ProxyLogic
import Darwin

private final class XPCReply<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Value, Error>?
    private var connection: NSXPCConnection?
    private var timeout: DispatchWorkItem?
    init(_ continuation: CheckedContinuation<Value, Error>, connection: NSXPCConnection, seconds: TimeInterval) {
        self.continuation = continuation; self.connection = connection
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
        timeout?.cancel(); waiting?.resume(with: result); connection?.invalidate()
    }
}

@MainActor
final class PrivilegedClient {
    static let shared = PrivilegedClient()
    private var installation: HelperInstallation?
    private var preparation: Task<Void, Error>?
    private var helperURL: URL { Bundle.main.bundleURL.appendingPathComponent("Contents/Library/HelperTools/SmartClashHelper") }
    private var coreURL: URL { Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/mihomo") }

    func prepare() async throws {
        if let preparation { return try await preparation.value }
        let task = Task { try await self.ensureAvailable() }
        preparation = task
        defer { preparation = nil }
        try await task.value
    }
    func setSystemProxy(_ snapshots: Data, enabled: Bool) async throws {
        _ = try ProxySnapshot.decodeValidated(snapshots)
        try await prepare()
        try await operation { $0.setSystemProxy(snapshots, enabled: enabled, withReply: $1) }
    }
    func startTun(_ configuration: Data) async throws {
        try TunConfiguration.validate(configuration)
        try await prepare()
        try Task.checkCancellation()
        try await operation { $0.startTun(configuration, withReply: $1) }
    }
    func stopTun() async throws {
        try await prepare()
        try await operation { $0.stopTun(withReply: $1) }
    }
    private func identity() throws -> HelperInstallation {
        if let installation { return installation }
        let value = try HelperInstallation(build: Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "development",
            ownerUID: getuid(), clientRequirement: CodeIdentity.requirement(at: Bundle.main.bundleURL),
            helperRequirement: CodeIdentity.requirement(at: helperURL), coreRequirement: CodeIdentity.requirement(at: coreURL))
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
        try await install(expected)
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
        // Administrator consent is used only to install/update root-owned, signed code.
        // Subsequent operations use a small, authenticated XPC interface; no shell is exposed over XPC.
        let script = """
        set -e
        umask 077
        /usr/bin/install -d -o root -g wheel -m 755 /Library/PrivilegedHelperTools \(q(HelperLocation.directory))
        /usr/bin/install -o root -g wheel -m 755 \(q(helperURL.path)) \(q(HelperLocation.executable + ".new"))
        /usr/bin/install -o root -g wheel -m 755 \(q(coreURL.path)) \(q(HelperLocation.directory + "/mihomo.new"))
        /usr/bin/codesign --verify --strict -R \(q(CodeIdentity.commandLineRequirement(installation.helperRequirement))) \(q(HelperLocation.executable + ".new"))
        /usr/bin/codesign --verify --strict -R \(q(CodeIdentity.commandLineRequirement(installation.coreRequirement))) \(q(HelperLocation.directory + "/mihomo.new"))
        if /bin/launchctl print system/\(HelperLocation.label) >/dev/null 2>&1; then /bin/launchctl bootout system/\(HelperLocation.label); fi
        /bin/mv -f \(q(HelperLocation.executable + ".new")) \(q(HelperLocation.executable))
        /bin/mv -f \(q(HelperLocation.directory + "/mihomo.new")) \(q(HelperLocation.directory + "/mihomo"))
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
    private func operation(_ send: @escaping (PrivilegedXPC, @escaping (String?) -> Void) -> Void) async throws {
        let _: Void = try await request(seconds: 120) { service, finish in
            send(service) { error in
                if let error { finish(.failure(ProxyError.message(error))) }
                else { finish(.success(())) }
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
