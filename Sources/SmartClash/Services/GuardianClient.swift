import Foundation
import Darwin
import LifecycleSupport
import CProcessIdentity

private final class GuardianChannel: @unchecked Sendable {
    private let lock = NSLock()
    private let gateLock = NSLock()
    private var terminating = false
    let process: Process
    let input: FileHandle
    let output: FileHandle
    private var buffer = Data()
    private var nextRequest = 0
    init(root: URL, nonce: String) throws {
        let executable = Bundle.main.bundleURL.appendingPathComponent("Contents/Library/HelperTools/SmartClashGuardian")
        guard FileManager.default.isExecutableFile(atPath: executable.path) else { throw LifecycleError.refused("安装包缺少 SmartClashGuardian，拒绝启动资源") }
        let child = Process(); child.executableURL = executable
        child.arguments = [root.path, nonce, try JSONEncoder().encode(ProcessIdentity.current()).base64EncodedString()]
        let commands = Pipe(), replies = Pipe()
        input = commands.fileHandleForWriting; output = replies.fileHandleForReading
        child.standardInput = commands; child.standardOutput = replies; child.standardError = FileHandle.nullDevice
        process = child
        _ = fcntl(input.fileDescriptor, F_SETNOSIGPIPE, 1)
        try child.run()
        try commands.fileHandleForReading.close(); try replies.fileHandleForWriting.close()
        let ready = try receive(seconds: 5)
        guard ready.id == "ready", ready.error == nil else { try? input.close(); throw LifecycleError.refused(ready.error ?? "守护未就绪") }
    }
    private func receive(seconds: TimeInterval) throws -> GuardianReply {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if let end = buffer.firstIndex(of: 10) {
                let line = buffer.prefix(upTo: end); buffer.removeSubrange(...end)
                return try JSONDecoder().decode(GuardianReply.self, from: line)
            }
            guard sc_readable(output.fileDescriptor, max(1, Int32(deadline.timeIntervalSinceNow * 1000))) > 0 else { break }
            var bytes = [UInt8](repeating: 0, count: 8192)
            let count = read(output.fileDescriptor, &bytes, bytes.count)
            guard count > 0 else { throw LifecycleError.refused("守护连接断开") }
            buffer.append(contentsOf: bytes.prefix(count))
            guard buffer.count < 1_000_000 else { throw LifecycleError.refused("守护响应过大") }
        }
        throw LifecycleError.refused("守护操作超时；恢复记录已保留")
    }
    func closeGate() { gateLock.lock(); terminating = true; gateLock.unlock() }
    func send(_ request: GuardianRequest) throws -> GuardianReply {
        lock.lock(); defer { lock.unlock() }
        let expected = request.operation == "end" ? "ended" : request.id
        // end 可能已完成但先前等待超时，重试时先读取尚未消费的最终确认。
        if request.operation == "end", !process.isRunning {
            let reply = try receive(seconds: 1)
            guard reply.id == expected, reply.error == nil else { throw LifecycleError.refused(reply.error ?? "守护结束未确认") }
            return reply
        }
        guard process.isRunning else { throw LifecycleError.refused("独立守护已退出，拒绝启动内核") }
        let data = try JSONEncoder().encode(request) + Data([10])
        if request.operation == "spawn" {
            gateLock.lock()
            defer { gateLock.unlock() }
            guard !terminating else { throw CancellationError() }
            try input.write(contentsOf: data)
        } else { try input.write(contentsOf: data) }
        let deadline = Date().addingTimeInterval(request.operation == "end" ? 8 : 5)
        while Date() < deadline {
            let reply = try receive(seconds: max(0.01, deadline.timeIntervalSinceNow))
            // 串行通道允许丢弃之前超时请求的迟到响应；不能把它当成本次确认。
            guard reply.id == expected else { continue }
            if let error = reply.error { throw LifecycleError.refused(error) }
            return reply
        }
        throw LifecycleError.refused("守护响应超时，恢复尚未确认")
    }
}

@MainActor
final class GuardianClient {
    static let shared = GuardianClient()
    private var channel: GuardianChannel?
    private var instance: InstanceLock?
    private let nonce = UUID().uuidString
    private var terminating = false
    private var children = Set<String>()
    var hasChildren: Bool { !children.isEmpty }
    var ready: Bool { channel != nil && !terminating }
    func bootstrap() throws {
        guard !terminating else { throw CancellationError() }
        if channel != nil { return }
        try Files.setup()
        let instance = try InstanceLock(path: Files.root.appendingPathComponent("instance.lock").path)
        // 独立守护清理期间，即使 GUI 的 instance.lock 已释放也不得初始化。
        do { let check = try InstanceLock(path: Files.root.appendingPathComponent("guardian.lock").path); withExtendedLifetime(check) {} }
        self.instance = instance
        do { channel = try GuardianChannel(root: Files.root, nonce: nonce) }
        catch { self.instance = nil; throw error }
    }
    func closeGate() { terminating = true; channel?.closeGate() }
    private func request(_ operation: String, id: String = UUID().uuidString, executable: String? = nil,
                         arguments: [String]? = nil, directory: String? = nil,
                         environment: [String: String]? = nil) async throws -> GuardianReply {
        guard let channel else { throw LifecycleError.refused("守护尚未 ready") }
        if operation == "spawn" { guard !terminating else { throw CancellationError() }; try Task.checkCancellation() }
        let message = GuardianRequest(nonce: nonce, operation: operation, id: id, executable: executable,
                                      arguments: arguments, directory: directory, environment: environment)
        return try await Task.detached { try channel.send(message) }.value
    }
    func spawn(executable: URL, arguments: [String], directory: URL,
               environment: [String: String] = [:]) async throws -> String {
        guard !terminating else { throw CancellationError() }
        try Task.checkCancellation()
        let id = UUID().uuidString
        children.insert(id)
        do {
            _ = try await request("spawn", id: id, executable: executable.path, arguments: arguments,
                                  directory: directory.path, environment: environment)
            if terminating || Task.isCancelled { throw CancellationError() }
            return id
        } catch { try? await stop(id); throw error }
    }
    func status(_ id: String) async throws -> GuardianReply { try await request("status", id: id) }
    func stop(_ id: String) async throws {
        _ = try await request("stop", id: id)
        children.remove(id)
    }
    func stopAll() async throws {
        var failures: [String] = []
        for id in Array(children) {
            do { try await stop(id) } catch { failures.append(error.localizedDescription) }
        }
        if !failures.isEmpty { throw LifecycleError.refused(failures.joined(separator: "；")) }
    }
    func validate(executable: URL, arguments: [String], directory: URL,
                  environment: [String: String] = [:]) async throws -> Int32 {
        let id = try await spawn(executable: executable, arguments: arguments, directory: directory, environment: environment)
        do {
            for _ in 0..<100 {
                try Task.checkCancellation()
                let status = try await status(id)
                if status.running == false { try await stop(id); return status.code ?? -1 }
                try await Task.sleep(nanoseconds: 100_000_000)
            }
            throw LifecycleError.refused("内核配置校验超时")
        } catch { try? await stop(id); throw error }
    }
    func end() async throws {
        closeGate()
        guard channel != nil else { return }
        _ = try await request("end")
        channel = nil; children.removeAll()
        // instance 锁保留至 GUI 实际退出；失败退出时不能允许重启初始化。
    }
}
