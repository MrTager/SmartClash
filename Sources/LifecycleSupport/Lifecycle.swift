import Foundation
import Darwin
import CProcessIdentity

public enum LifecycleError: LocalizedError {
    case refused(String)
    public var errorDescription: String? { if case let .refused(text) = self { return text }; return nil }
}

public struct ProcessIdentity: Codable, Equatable, Sendable {
    public let pid: Int32
    public let uid: UInt32
    public let seconds: UInt64
    public let micros: UInt64
    public let executable: String
    public init(pid: Int32, uid: UInt32, seconds: UInt64, micros: UInt64, executable: String) {
        self.pid = pid; self.uid = uid; self.seconds = seconds; self.micros = micros; self.executable = executable
    }
    public static func query(_ pid: Int32) -> ProcessObservation {
        var uid: UInt32 = 0, seconds: UInt64 = 0, micros: UInt64 = 0
        var path = [CChar](repeating: 0, count: 4096)
        let result = sc_process_identity(pid, &uid, &seconds, &micros, &path, UInt32(path.count))
        if result == 0 { return .absent }
        guard result == 1 else { return .unknown }
        return .present(.init(pid: pid, uid: uid, seconds: seconds, micros: micros, executable: String(cString: path)))
    }
    public static func current() throws -> Self {
        guard case let .present(identity) = query(getpid()) else { throw LifecycleError.refused("无法核实本进程启动身份") }
        return identity
    }
}
public enum ProcessObservation: Equatable, Sendable { case present(ProcessIdentity), absent, unknown }
public enum OwnershipDecision: Equatable { case same, gone, unknown }
public enum Ownership {
    public static func compare(_ expected: ProcessIdentity, _ observation: ProcessObservation) -> OwnershipDecision {
        switch observation {
        case .absent: return .gone
        case .unknown: return .unknown
        case .present(let actual): return actual == expected ? .same : .gone
        }
    }
}

// GUI 和 guardian 继承同一打开文件描述符；任一仍存活时都不能接管。
public final class InstanceLock: @unchecked Sendable {
    public let descriptor: Int32
    public init(path: String) throws {
        descriptor = open(path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw LifecycleError.refused("无法打开单实例锁") }
        var info = stat()
        guard fstat(descriptor, &info) == 0, info.st_uid == getuid(),
              (info.st_mode & S_IFMT) == S_IFREG, info.st_nlink == 1,
              flock(descriptor, LOCK_EX | LOCK_NB) == 0 else {
            close(descriptor); throw LifecycleError.refused("另一个实例或守护清理仍在运行")
        }
        _ = fchmod(descriptor, 0o600)
    }
    deinit { close(descriptor) }
}

public enum DurableFile {
    public static func write<T: Encodable>(_ value: T, to url: URL) throws {
        let data = try JSONEncoder().encode(value)
        let temporary = url.deletingLastPathComponent().appendingPathComponent(".journal-" + UUID().uuidString)
        let fd = open(temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard fd >= 0 else { throw LifecycleError.refused("无法创建恢复日志") }
        defer { close(fd); try? FileManager.default.removeItem(at: temporary) }
        try data.withUnsafeBytes { bytes in
            var offset = 0
            while offset < bytes.count {
                let count = Darwin.write(fd, bytes.baseAddress!.advanced(by: offset), bytes.count - offset)
                guard count > 0 else { throw LifecycleError.refused("恢复日志写入失败") }
                offset += count
            }
        }
        guard fsync(fd) == 0, rename(temporary.path, url.path) == 0 else { throw LifecycleError.refused("恢复日志持久化失败") }
        let parent = open(url.deletingLastPathComponent().path, O_RDONLY | O_CLOEXEC)
        defer { if parent >= 0 { close(parent) } }
        guard parent >= 0, fsync(parent) == 0 else { throw LifecycleError.refused("恢复目录同步失败") }
    }
}

public final class SupervisedChild {
    public let pid: Int32
    public let monitorPID: Int32
    private let channel: Int32
    public init(executable: String, arguments: [String], directory: String, log: Int32, lock: Int32) throws {
        let strings = ([executable] + arguments).map { strdup($0) }
        defer { strings.forEach { free($0) } }
        var argv = strings + [nil]
        var monitor: Int32 = 0, core: Int32 = 0, socket: Int32 = -1
        let result = argv.withUnsafeMutableBufferPointer { buffer in
            sc_supervised_spawn(executable, buffer.baseAddress, directory, log, lock, &monitor, &core, &socket)
        }
        guard result == 0 else { throw LifecycleError.refused("无法建立独立内核监督") }
        pid = core; monitorPID = monitor; channel = socket
    }
    public func release() throws {
        guard sc_supervised_release(channel) == 0 else { throw LifecycleError.refused("内核执行闸门未确认") }
    }
    public func status() throws -> (running: Bool, code: Int32) {
        var running: Int32 = 0, code: Int32 = -1
        guard sc_supervised_status(channel, &running, &code) == 0 else { throw LifecycleError.refused("内核监督失联，保留日志") }
        return (running != 0, code)
    }
    public func stop() throws {
        guard sc_supervised_stop(channel) == 0 else { throw LifecycleError.refused("内核停止未确认") }
    }
    deinit { close(channel); var status: Int32 = 0; _ = waitpid(monitorPID, &status, WNOHANG) }
}

public final class TerminationGate: @unchecked Sendable {
    private let lock = NSLock()
    private var closed = false
    public init() {}
    public var isClosed: Bool { lock.lock(); defer { lock.unlock() }; return closed }
    public func close() { lock.lock(); closed = true; lock.unlock() }
    public func check() throws { if isClosed { throw LifecycleError.refused("应用正在退出，拒绝后续操作") } }
}
