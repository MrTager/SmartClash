import Foundation
import PrivilegedSupport
import ProxyLogic
import LifecycleSupport
import Darwin

final class NetworkBackend: HelperSessionBackend {
    private var preferences: ProxyPreferences?
    var session: () -> String? = { nil }
    func snapshot(port: Int) throws -> [ProxyServiceChange] {
        guard Configuration.validateProxyPort(port) == nil else { throw LifecycleError.refused("代理端口无效") }
        let transaction = try ProxyPreferences()
        let changes = try transaction.snapshot(port: port)
        preferences = transaction
        return changes
    }
    func apply(_ changes: [ProxyServiceChange]) throws {
        defer { preferences = nil }
        guard let preferences else { throw LifecycleError.refused("缺少代理事务") }
        try preferences.apply(changes)
    }
    func restore(_ changes: [ProxyServiceChange]) throws -> (remaining: [ProxyServiceChange], reports: [String]) {
        preferences = nil
        return try ProxyPreferences().restore(changes)
    }
    func startTun(_ data: Data, kernel: String, port: Int) throws {
        guard let kind = KernelKind(rawValue: kernel), kind.capabilities.tun, let nonce = session() else { throw LifecycleError.refused("TUN 内核或会话无效") }
        try TunConfiguration.validate(data, kernel: kind, port: port)
        guard !FileManager.default.fileExists(atPath: HelperLocation.corePlist), try !loaded() else { throw LifecycleError.refused("发现未知 TUN 服务，保留现场") }
        let root = HelperLocation.coreDirectory
        try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let core = HelperLocation.directory + "/" + kind.binaryName
        let installation = try PropertyListDecoder().decode(HelperInstallation.self, from: Data(contentsOf: URL(fileURLWithPath: HelperLocation.manifest)))
        guard let requirement = installation.kernelRequirements?[kind.binaryName] else { throw LifecycleError.refused("安装记录缺少内核签名") }
        try run("/usr/bin/codesign", ["--verify", "--strict", "-R", CodeIdentity.commandLineRequirement(requirement), core])
        let config = root + "/config.json"
        try write(data, path: config)
        let arguments = kind == .mihomo ? [core, "-d", root, "-f", config] : [core, "run", "-c", config, "-D", root]
        // 不派生 root 校验内核；配置已通过普通 guardian 校验及严格 schema 校验。
        let plist: [String: Any] = ["Label": HelperLocation.coreLabel, "SmartClashSession": nonce,
            "ProgramArguments": arguments, "RunAtLoad": true, "KeepAlive": false,
            "StandardOutPath": root + "/core.log", "StandardErrorPath": root + "/core.log",
            "WorkingDirectory": root, "Umask": 0o077]
        try write(PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0), path: HelperLocation.corePlist)
        try run("/bin/launchctl", ["bootstrap", "system", HelperLocation.corePlist])
    }
    private func loaded() throws -> Bool {
        let result = try command("/bin/launchctl", ["print", "system/" + HelperLocation.coreLabel])
        if result.code == 0 { return true }
        guard result.text.contains("Could not find service") else { throw LifecycleError.refused("无法确认 TUN launchd 状态，保留恢复记录") }
        return false
    }
    func stopTun() throws {
        let exists = FileManager.default.fileExists(atPath: HelperLocation.corePlist)
        if exists {
            let object = try PropertyListSerialization.propertyList(from: Data(contentsOf: URL(fileURLWithPath: HelperLocation.corePlist)), format: nil) as? [String: Any]
            guard let object, object["Label"] as? String == HelperLocation.coreLabel,
                  object["SmartClashSession"] as? String == session(),
                  let args = object["ProgramArguments"] as? [String],
                  ["mihomo", "sing-box"].map({ HelperLocation.directory + "/" + $0 }).contains(args.first ?? "") else {
                throw LifecycleError.refused("TUN 服务不属于本会话，未删除")
            }
        }
        if try loaded() {
            guard exists else { throw LifecycleError.refused("未知 TUN label，没有本会话凭证，未停止") }
            try run("/bin/launchctl", ["bootout", "system/" + HelperLocation.coreLabel])
            guard try !loaded() else { throw LifecycleError.refused("TUN 服务仍存在") }
        }
        if exists { try FileManager.default.removeItem(atPath: HelperLocation.corePlist) }
        // 保留 root 工作目录供审计；绝不递归删除未知目录内容。
    }
    private func write(_ data: Data, path: String) throws {
        try data.write(to: URL(fileURLWithPath: path), options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
    }
    private func command(_ executable: String, _ arguments: [String]) throws -> (code: Int32, text: String) {
        let child = Process(); child.executableURL = URL(fileURLWithPath: executable); child.arguments = arguments
        child.environment = ["PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "LC_ALL": "C", "HOME": "/var/root"]
        let pipe = Pipe(); child.standardOutput = pipe; child.standardError = pipe
        try child.run()
        let timeout = DispatchWorkItem { if child.isRunning { child.terminate() } }
        DispatchQueue.global().asyncAfter(deadline: .now() + 5, execute: timeout)
        let killTimeout = DispatchWorkItem { if child.isRunning { kill(child.processIdentifier, SIGKILL) } }
        DispatchQueue.global().asyncAfter(deadline: .now() + 6, execute: killTimeout)
        let bytes = pipe.fileHandleForReading.readDataToEndOfFile(); child.waitUntilExit()
        timeout.cancel(); killTimeout.cancel()
        return (child.terminationStatus, String(decoding: bytes, as: UTF8.self))
    }
    private func run(_ executable: String, _ arguments: [String]) throws {
        guard try command(executable, arguments).code == 0 else { throw LifecycleError.refused("后台固定网络操作失败：" + URL(fileURLWithPath: executable).lastPathComponent) }
    }
}

// 每条 XPC 连接有独立 endpoint。只有显式 beginSession 的长连接断开才触发回收。
final class SessionEndpoint: NSObject, PrivilegedXPC {
    unowned let service: HelperService
    let owner: ProcessIdentity
    let id = UUID()
    var nonce: String?
    init(service: HelperService, owner: ProcessIdentity) { self.service = service; self.owner = owner }
    func ping(withReply reply: @escaping (Int) -> Void) { reply(HelperLocation.version) }
    private func execute(_ reply: @escaping (String?) -> Void, _ operation: @escaping () throws -> Void) {
        service.queue.async { do { try operation(); reply(nil) } catch { reply(error.localizedDescription) } }
    }
    private func authorize(_ nonce: String, cleanup: Bool = false) throws {
        guard self.nonce == nonce, service.active == id else { throw LifecycleError.refused("必须使用专用长会话连接") }
        try service.controller.authorize(nonce: nonce, owner: owner, cleanup: cleanup)
    }
    func beginSession(_ nonce: String, withReply reply: @escaping (String?) -> Void) {
        execute(reply) {
            guard self.service.active == nil || self.service.active == self.id else { throw LifecycleError.refused("已有网络会话") }
            try self.service.controller.begin(nonce: nonce, owner: self.owner)
            self.nonce = nonce; self.service.active = self.id
        }
    }
    func heartbeat(_ nonce: String, withReply reply: @escaping (String?) -> Void) { execute(reply) { try self.authorize(nonce) } }
    func endSession(_ nonce: String, withReply reply: @escaping (String?) -> Void) {
        execute(reply) { try self.authorize(nonce, cleanup: true); try self.service.controller.cleanup(); self.service.active = nil; self.nonce = nil }
    }
    func status(_ nonce: String, withReply reply: @escaping (Data?, String?) -> Void) {
        service.queue.async {
            do { try self.authorize(nonce, cleanup: true); reply(try JSONEncoder().encode(HelperSessionStatus(self.service.controller.journal)), nil) }
            catch { reply(nil, error.localizedDescription) }
        }
    }
    func setSystemProxy(_ nonce: String, port: Int, enabled: Bool, withReply reply: @escaping (String?) -> Void) {
        execute(reply) {
            try self.authorize(nonce, cleanup: !enabled)
            guard Configuration.validateProxyPort(port) == nil else { throw LifecycleError.refused("代理端口无效") }
            try self.service.controller.setProxy(port: port, enabled: enabled)
        }
    }
    func startTun(_ configuration: Data, kernel: String, port: Int, session: String, withReply reply: @escaping (String?) -> Void) {
        execute(reply) { try self.authorize(session); try self.service.controller.startTun(configuration, kernel: kernel, port: port) }
    }
    func stopTun(_ nonce: String, withReply reply: @escaping (String?) -> Void) {
        execute(reply) { try self.authorize(nonce, cleanup: true); try self.service.controller.stopTun() }
    }
    func disconnected() {
        service.queue.async {
            guard self.service.active == self.id, self.nonce != nil else { return }
            self.service.active = nil
            do { try self.service.controller.cleanup() } catch { fputs("会话断连恢复未完成：\(error.localizedDescription)\n", stderr) }
        }
    }
}

final class HelperService: NSObject, NSXPCListenerDelegate {
    let queue = DispatchQueue(label: "com.smartclash.helper.operations")
    let controller: HelperSessionController
    let installation: HelperInstallation
    var active: UUID?
    private var timer: DispatchSourceTimer?
    init(installation: HelperInstallation) throws {
        self.installation = installation
        try FileManager.default.createDirectory(atPath: HelperLocation.sessionDirectory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let url = URL(fileURLWithPath: HelperLocation.journal)
        let journal = FileManager.default.fileExists(atPath: url.path) ? try JSONDecoder().decode(HelperSessionJournal.self, from: Data(contentsOf: url)) : nil
        let backend = NetworkBackend()
        controller = HelperSessionController(journal: journal, backend: backend, save: { try DurableFile.write($0, to: url) })
        super.init()
        backend.session = { [weak self] in self?.controller.journal?.nonce }
        // 重启不接管旧会话，即便 GUI 还活着，也先恢复 helper 自己的资源。
        do { try controller.reconcile() } catch { fputs("启动对账未完成：\(error.localizedDescription)\n", stderr) }
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + 1, repeating: 1)
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            do { try self.controller.checkOwner() } catch { }
        }
        self.timer = timer; timer.resume()
    }
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        // processIdentifier / effectiveUserIdentifier 由 XPC 审计身份提供，不接受客户端传入 PID。
        guard connection.effectiveUserIdentifier == installation.ownerUID,
              case let .present(owner) = ProcessIdentity.query(connection.processIdentifier), owner.uid == installation.ownerUID else { return false }
        connection.setCodeSigningRequirement(installation.clientRequirement)
        let endpoint = SessionEndpoint(service: self, owner: owner)
        connection.exportedInterface = NSXPCInterface(with: PrivilegedXPC.self); connection.exportedObject = endpoint
        connection.invalidationHandler = { endpoint.disconnected() }
        connection.interruptionHandler = { endpoint.disconnected() }
        connection.resume(); return true
    }
}

do {
    guard getuid() == 0 else { throw LifecycleError.refused("后台服务只能由系统 launchd 启动") }
    let installation = try PropertyListDecoder().decode(HelperInstallation.self, from: Data(contentsOf: URL(fileURLWithPath: HelperLocation.manifest)))
    guard installation.version == HelperLocation.version, installation.ownerUID > 0 else { throw LifecycleError.refused("后台安装记录无效") }
    try CodeIdentity.validate(installation.clientRequirement)
    let delegate = try HelperService(installation: installation)
    let listener = NSXPCListener(machServiceName: HelperLocation.label)
    listener.setConnectionCodeSigningRequirement(installation.clientRequirement)
    listener.delegate = delegate; listener.resume()
    withExtendedLifetime(delegate) { RunLoop.current.run() }
} catch { fputs("SmartClashHelper: \(error.localizedDescription)\n", stderr); exit(1) }
