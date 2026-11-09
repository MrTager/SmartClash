import Foundation
import Darwin
import LifecycleSupport
import CProcessIdentity

// 仅同用户执行，不接受任何 root 模式或提权入口。
final class Guardian {
    let root: URL
    let nonce: String
    let owner: ProcessIdentity
    let lock: InstanceLock
    let journalURL: URL
    var journal: GuardianJournal
    var children: [String: SupervisedChild] = [:]
    var logs: [String: FileHandle] = [:]
    init() throws {
        guard getuid() != 0, CommandLine.arguments.count == 4 else { throw LifecycleError.refused("守护参数无效") }
        root = URL(fileURLWithPath: CommandLine.arguments[1]).standardizedFileURL
        nonce = CommandLine.arguments[2]
        guard let data = Data(base64Encoded: CommandLine.arguments[3]) else { throw LifecycleError.refused("缺少启动身份") }
        owner = try JSONDecoder().decode(ProcessIdentity.self, from: data)
        guard owner.pid == getppid(), owner.uid == getuid(), Ownership.compare(owner, ProcessIdentity.query(owner.pid)) == .same else {
            throw LifecycleError.refused("守护 owner 身份不匹配")
        }
        lock = try InstanceLock(path: root.appendingPathComponent("guardian.lock").path)
        journalURL = root.appendingPathComponent("guardian-session.json")
        if FileManager.default.fileExists(atPath: journalURL.path) {
            let old = try JSONDecoder().decode(GuardianJournal.self, from: Data(contentsOf: journalURL))
            guard old.phase == "ended" || Ownership.compare(old.owner, ProcessIdentity.query(old.owner.pid)) == .gone else {
                throw LifecycleError.refused("旧会话仍存活或身份未知，保留记录")
            }
            for identity in old.children.values {
                switch Ownership.compare(identity, ProcessIdentity.query(identity.pid)) {
                case .gone: continue
                case .unknown: throw LifecycleError.refused("遗留子进程身份未知，未发送信号")
                case .same:
                    guard identity.uid == getuid() else { throw LifecycleError.refused("遗留进程用户不匹配") }
                    // 遗留非直接子进程不以 PID 查询后发信号，避免 TOCTOU 误杀。
                    throw LifecycleError.refused("遗留内核仍存活，需人工核实，未发送信号")
                }
            }
        }
        journal = GuardianJournal(nonce: nonce, owner: owner)
        try persist()
    }
    func persist() throws { try DurableFile.write(journal, to: journalURL) }
    func respond(_ reply: GuardianReply) {
        if let data = try? JSONEncoder().encode(reply) { FileHandle.standardOutput.write(data + Data([10])) }
    }
    func stop(_ id: String) throws {
        guard let child = children[id] else { return }
        try child.stop()
        children[id] = nil; try? logs.removeValue(forKey: id)?.close()
        journal.children[id] = nil; try persist()
    }
    func handle(_ request: GuardianRequest) throws -> Bool {
        guard request.nonce == nonce else { throw LifecycleError.refused("会话 nonce 不匹配") }
        guard Ownership.compare(owner, ProcessIdentity.query(owner.pid)) == .same else { return false }
        switch request.operation {
        case "heartbeat": respond(.init(id: request.id))
        case "spawn":
            guard children.count < 32, children[request.id] == nil, let executable = request.executable,
                  let directory = request.directory, (request.arguments?.count ?? 0) < 128 else { throw LifecycleError.refused("守护请求无效") }
            // 守护无 root 权限；生产只接受同一安装包里的固定内核。
            let own = try ProcessIdentity.current().executable
            let bundle = URL(fileURLWithPath: own).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            let allowed = ["mihomo", "sing-box", "xray"].map { bundle.appendingPathComponent("Contents/Resources/" + $0).resolvingSymlinksInPath().path }
            let canonical = URL(fileURLWithPath: executable).resolvingSymlinksInPath().path
            let fixture = ProcessInfo.processInfo.environment["SMARTCLASH_GUARDIAN_FIXTURE"] == "1" && canonical == "/bin/sleep"
            guard allowed.contains(canonical) || fixture,
                  URL(fileURLWithPath: directory).resolvingSymlinksInPath().path.hasPrefix(root.resolvingSymlinksInPath().path + "/") || directory == root.path else {
                throw LifecycleError.refused("拒绝包外内核或工作目录")
            }
            journal.phase = "spawning"; try persist()
            let log = root.appendingPathComponent("guardian-" + request.id + ".log")
            guard FileManager.default.createFile(atPath: log.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { throw LifecycleError.refused("无法创建内核日志") }
            let handle = try FileHandle(forWritingTo: log); logs[request.id] = handle
            do {
                let child = try SupervisedChild(executable: canonical, arguments: request.arguments ?? [],
                    directory: directory, log: handle.fileDescriptor, lock: lock.descriptor)
                children[request.id] = child
                guard case let .present(identity) = ProcessIdentity.query(child.pid) else {
                    throw LifecycleError.refused("未能记录子进程启动身份")
                }
                journal.children[request.id] = identity
                // 此时内核阻塞在 C 执行闸门；只有日志持久化后才能 exec。
                try persist(); try child.release()
                guard case let .present(executed) = ProcessIdentity.query(child.pid) else {
                    let status = try child.status()
                    if !status.running { journal.phase = "ready"; try persist(); respond(.init(id: request.id, pid: child.pid, running: false, code: status.code)); return true }
                    throw LifecycleError.refused("内核 exec 身份未知")
                }
                journal.children[request.id] = executed; journal.phase = "ready"; try persist()
                respond(.init(id: request.id, pid: child.pid, running: true))
            } catch {
                try stop(request.id); journal.phase = "ready"; try persist(); throw error
            }
        case "status":
            let status = try children[request.id]?.status()
            respond(.init(id: request.id, running: status?.running ?? false, code: status?.code))
        case "stop": try stop(request.id); respond(.init(id: request.id, running: false))
        case "end": return false
        default: throw LifecycleError.refused("未知守护操作")
        }
        return true
    }
    func run() -> Never {
        respond(.init(id: "ready"))
        var buffer = Data()
        while true {
            // 不因睡眠或短暂 lease 超时回收；仅 EOF 或已核实 owner 死亡触发。
            if Ownership.compare(owner, ProcessIdentity.query(owner.pid)) == .gone { break }
            let ready = sc_readable(STDIN_FILENO, 1000)
            if ready < 0 { if errno == EINTR { continue }; break }
            if ready == 0 { continue }
            var bytes = [UInt8](repeating: 0, count: 8192)
            let count = read(STDIN_FILENO, &bytes, bytes.count)
            if count <= 0 { break }
            buffer.append(contentsOf: bytes.prefix(count))
            if buffer.count > 16_000_000 { break }
            while let end = buffer.firstIndex(of: 10) {
                let line = buffer.prefix(upTo: end); buffer.removeSubrange(...end)
                do {
                    let request = try JSONDecoder().decode(GuardianRequest.self, from: line)
                    do { if try !handle(request) { cleanup() } }
                    catch { respond(.init(id: request.id, error: error.localizedDescription)) }
                } catch { cleanup() }
            }
        }
        cleanup()
    }
    func cleanup() -> Never {
        journal.phase = "cleaning"; try? persist()
        var failed = false
        for id in Array(children.keys) { do { try stop(id) } catch { failed = true } }
        if !failed { journal.phase = "ended"; try? persist(); respond(.init(id: "ended")); exit(0) }
        // 清理失败时继续持有锁，防止新实例误接管。
        respond(.init(id: "ended", error: "守护清理未完成，保留恢复记录"))
        RunLoop.current.run(); exit(1)
    }
}
signal(SIGPIPE, SIG_IGN)
do { try Guardian().run() }
catch {
    let reply = GuardianReply(id: "ready", error: error.localizedDescription)
    if let data = try? JSONEncoder().encode(reply) { FileHandle.standardOutput.write(data + Data([10])) }
    exit(1)
}
