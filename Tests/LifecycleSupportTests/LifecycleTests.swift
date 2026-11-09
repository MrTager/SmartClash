import XCTest
import Foundation
import Darwin
import LifecycleSupport

final class LifecycleTests: XCTestCase {
    private let identity = ProcessIdentity(pid: 42001, uid: 501, seconds: 100, micros: 123, executable: "/bin/sleep")

    private func temporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("SmartClash-lifecycle-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        addTeardownBlock { try FileManager.default.removeItem(at: directory) }
        return directory
    }

    func testProcessIdentityCodableRoundTripPreservesEveryOwnershipField() throws {
        let decoded = try JSONDecoder().decode(ProcessIdentity.self, from: JSONEncoder().encode(identity))
        XCTAssertEqual(decoded, identity)
        XCTAssertEqual(Ownership.compare(identity, .present(decoded)), .same)
    }

    func testOwnershipRejectsReusedPIDAndAnyChangedIdentityField() {
        let replacements = [
            ProcessIdentity(pid: identity.pid, uid: identity.uid, seconds: 101, micros: identity.micros, executable: identity.executable),
            ProcessIdentity(pid: identity.pid, uid: identity.uid, seconds: identity.seconds, micros: 124, executable: identity.executable),
            ProcessIdentity(pid: identity.pid, uid: 502, seconds: identity.seconds, micros: identity.micros, executable: identity.executable),
            ProcessIdentity(pid: identity.pid, uid: identity.uid, seconds: identity.seconds, micros: identity.micros, executable: "/usr/bin/true"),
            ProcessIdentity(pid: 42002, uid: identity.uid, seconds: identity.seconds, micros: identity.micros, executable: identity.executable)
        ]
        for replacement in replacements {
            XCTAssertEqual(Ownership.compare(identity, .present(replacement)), .gone, "身份变化不能因 PID 相同而被接受")
        }
    }

    func testOwnershipDistinguishesUnknownFromConfirmedAbsence() {
        XCTAssertEqual(Ownership.compare(identity, .absent), .gone)
        XCTAssertEqual(Ownership.compare(identity, .unknown), .unknown)
        for pid: Int32 in [-1, 0, 1] {
            XCTAssertEqual(ProcessIdentity.query(pid), .unknown, "不可核验的 PID 不能视为确定死亡")
        }
    }

    func testCurrentIdentityMatchesOnlyThisUserAndProcess() throws {
        let current = try ProcessIdentity.current()
        XCTAssertEqual(current.pid, getpid())
        XCTAssertEqual(current.uid, getuid())
        XCTAssertGreaterThan(current.seconds, 0)
        XCTAssertFalse(current.executable.isEmpty)
        XCTAssertEqual(ProcessIdentity.query(getpid()), .present(current))
    }

    func testTerminationGateRemainsPermanentlyClosedAfterRepeatedCloseAndCheck() throws {
        let gate = TerminationGate()
        XCTAssertFalse(gate.isClosed)
        try gate.check()
        for _ in 0..<20 {
            gate.close()
            XCTAssertTrue(gate.isClosed)
            XCTAssertThrowsError(try gate.check()) { error in
                guard case LifecycleError.refused = error else { return XCTFail("关闭后应拒绝后续操作") }
            }
        }
    }

    func testTerminationGateClosedStateIsVisibleToConcurrentCallers() {
        let gate = TerminationGate()
        gate.close()
        let group = DispatchGroup()
        for _ in 0..<16 {
            group.enter()
            DispatchQueue.global().async {
                defer { group.leave() }
                gate.close()
                XCTAssertTrue(gate.isClosed)
                XCTAssertThrowsError(try gate.check())
            }
        }
        XCTAssertEqual(group.wait(timeout: .now() + 5), .success)
    }

    private struct Payload: Codable, Equatable {
        let generation: Int
        let text: String
    }

    func testDurableFileReplacesAtomicallyWithoutChangingAnAlreadyOpenFile() throws {
        let directory = try temporaryDirectory()
        let url = directory.appendingPathComponent("journal.json")
        let first = Payload(generation: 1, text: String(repeating: "旧", count: 4096))
        let second = Payload(generation: 2, text: String(repeating: "新", count: 8192))
        try DurableFile.write(first, to: url)
        let originalBytes = try Data(contentsOf: url)
        let original = try FileHandle(forReadingFrom: url)
        defer { try? original.close() }
        var before = stat()
        XCTAssertEqual(fstat(original.fileDescriptor, &before), 0)

        try DurableFile.write(second, to: url)

        var after = stat()
        XCTAssertEqual(lstat(url.path, &after), 0)
        XCTAssertNotEqual(before.st_ino, after.st_ino, "必须替换目录项，而不是原地截断旧文件")
        XCTAssertEqual(try original.readToEnd(), originalBytes, "已有读者仍应看到完整旧版本")
        XCTAssertEqual(try JSONDecoder().decode(Payload.self, from: Data(contentsOf: url)), second)
        XCTAssertEqual(after.st_uid, getuid())
        XCTAssertEqual(after.st_mode & 0o777, 0o600)
        XCTAssertEqual(after.st_mode & S_IFMT, S_IFREG)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path), ["journal.json"])
    }

    private struct Unencodable: Encodable {
        func encode(to encoder: Encoder) throws { throw LifecycleError.refused("编码失败测试桩") }
    }

    func testDurableFileEncodingFailurePreservesThePreviousJournal() throws {
        let directory = try temporaryDirectory()
        let url = directory.appendingPathComponent("journal.json")
        try DurableFile.write(Payload(generation: 1, text: "保留旧日志"), to: url)
        let original = try Data(contentsOf: url)
        XCTAssertThrowsError(try DurableFile.write(Unencodable(), to: url))
        XCTAssertEqual(try Data(contentsOf: url), original)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path), ["journal.json"])
    }

    func testDurableFileFailedRenameRemovesOnlyItsTemporaryFile() throws {
        let directory = try temporaryDirectory()
        let destination = directory.appendingPathComponent("existing-directory")
        try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: false)
        let marker = destination.appendingPathComponent("fixture")
        try Data("不可覆盖".utf8).write(to: marker)
        XCTAssertThrowsError(try DurableFile.write(Payload(generation: 1, text: "新日志"), to: destination))
        XCTAssertEqual(try Data(contentsOf: marker), Data("不可覆盖".utf8))
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path), ["existing-directory"])
    }

    func testInstanceLockExcludesAnotherOpenAndCanBeAcquiredAfterRelease() throws {
        let path = try temporaryDirectory().appendingPathComponent("instance.lock").path
        var first: InstanceLock? = try InstanceLock(path: path)
        XCTAssertThrowsError(try InstanceLock(path: path))
        XCTAssertGreaterThanOrEqual(first!.descriptor, 0)
        first = nil
        let second = try InstanceLock(path: path)
        withExtendedLifetime(second) {
            XCTAssertThrowsError(try InstanceLock(path: path))
        }
    }

    func testInstanceLockRejectsSymlinkWithoutChangingItsTarget() throws {
        let directory = try temporaryDirectory()
        let target = directory.appendingPathComponent("fixture")
        let link = directory.appendingPathComponent("instance.lock")
        let bytes = Data("同用户测试文件".utf8)
        try bytes.write(to: target)
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: target)
        XCTAssertThrowsError(try InstanceLock(path: link.path))
        XCTAssertEqual(try Data(contentsOf: target), bytes)
    }

    private func requireSleepFixture() throws {
        guard getuid() != 0 else { throw XCTSkip("子进程测试仅以普通用户运行，不启动特权进程") }
        guard FileManager.default.isExecutableFile(atPath: "/bin/sleep") else {
            throw XCTSkip("系统没有 /bin/sleep 测试程序")
        }
    }

    private func runningSleepIdentity(_ pid: Int32) async throws -> ProcessIdentity {
        let path = URL(fileURLWithPath: "/bin/sleep").resolvingSymlinksInPath().path
        let deadline = ProcessInfo.processInfo.systemUptime + 5
        while ProcessInfo.processInfo.systemUptime < deadline {
            if case let .present(actual) = ProcessIdentity.query(pid),
               URL(fileURLWithPath: actual.executable).resolvingSymlinksInPath().path == path {
                XCTAssertEqual(actual.uid, getuid(), "只允许同用户测试子进程")
                return actual
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTFail("监督子进程未执行 /bin/sleep")
        throw LifecycleError.refused("等待系统测试程序执行超时")
    }

    // 只等待本测试创建的直接子进程；绝不根据日志 PID 发信号。
    private func reapMonitor(_ pid: Int32, file: StaticString = #filePath, line: UInt = #line) {
        let deadline = ProcessInfo.processInfo.systemUptime + 5
        while ProcessInfo.processInfo.systemUptime < deadline {
            var status: Int32 = 0
            let result = waitpid(pid, &status, WNOHANG)
            if result == pid {
                XCTAssertEqual(status, 0, "监督进程应正常完成回收", file: file, line: line)
                return
            }
            if result == -1 {
                if errno == EINTR { continue }
                XCTAssertEqual(errno, ECHILD, "监督进程应已由 SupervisedChild.deinit 回收", file: file, line: line)
                return
            }
            usleep(10_000)
        }
        XCTFail("关闭通道后监督进程未在期限内退出", file: file, line: line)
    }

    func testSupervisedSleepRunsAndStopConfirmsChildExit() async throws {
        try requireSleepFixture()
        let directory = try temporaryDirectory()
        let instance = try InstanceLock(path: directory.appendingPathComponent("instance.lock").path)
        let log = open("/dev/null", O_WRONLY | O_CLOEXEC)
        XCTAssertGreaterThanOrEqual(log, 0)
        guard log >= 0 else { throw LifecycleError.refused("无法打开测试日志") }
        defer { close(log) }
        var child: SupervisedChild? = try SupervisedChild(executable: "/bin/sleep", arguments: ["30"],
            directory: directory.path, log: log, lock: instance.descriptor)
        let monitor = child!.monitorPID
        defer { child = nil; reapMonitor(monitor); withExtendedLifetime(instance) {} }
        XCTAssertNotEqual(child!.pid, monitor)
        try child!.release()
        let running = try await runningSleepIdentity(child!.pid)
        XCTAssertTrue(try child!.status().running)
        try child!.stop()
        child = nil
        reapMonitor(monitor)
        XCTAssertEqual(Ownership.compare(running, ProcessIdentity.query(running.pid)), .gone)
    }

    func testClosingReleasedChildChannelReclaimsSleepMonitorAndInheritedLock() async throws {
        try requireSleepFixture()
        let directory = try temporaryDirectory()
        let path = directory.appendingPathComponent("instance.lock").path
        var instance: InstanceLock? = try InstanceLock(path: path)
        let log = open("/dev/null", O_WRONLY | O_CLOEXEC)
        XCTAssertGreaterThanOrEqual(log, 0)
        guard log >= 0 else { throw LifecycleError.refused("无法打开测试日志") }
        defer { close(log) }
        var child: SupervisedChild? = try SupervisedChild(executable: "/bin/sleep", arguments: ["30"],
            directory: directory.path, log: log, lock: instance!.descriptor)
        let monitor = child!.monitorPID
        defer { child = nil; reapMonitor(monitor) }
        try child!.release()
        let running = try await runningSleepIdentity(child!.pid)
        XCTAssertTrue(try child!.status().running)
        instance = nil
        XCTAssertThrowsError(try InstanceLock(path: path), "监督进程存活时必须继续持有继承的锁")

        child = nil
        reapMonitor(monitor)

        XCTAssertEqual(Ownership.compare(running, ProcessIdentity.query(running.pid)), .gone)
        let replacement = try InstanceLock(path: path)
        withExtendedLifetime(replacement) {}
    }

    func testClosingUnreleasedChildChannelAlsoReclaimsTheGatedChild() throws {
        try requireSleepFixture()
        let directory = try temporaryDirectory()
        let instance = try InstanceLock(path: directory.appendingPathComponent("instance.lock").path)
        let log = open("/dev/null", O_WRONLY | O_CLOEXEC)
        XCTAssertGreaterThanOrEqual(log, 0)
        guard log >= 0 else { throw LifecycleError.refused("无法打开测试日志") }
        defer { close(log) }
        var child: SupervisedChild? = try SupervisedChild(executable: "/bin/sleep", arguments: ["30"],
            directory: directory.path, log: log, lock: instance.descriptor)
        let monitor = child!.monitorPID
        let pid = child!.pid
        XCTAssertTrue(try child!.status().running)
        child = nil
        reapMonitor(monitor)
        XCTAssertEqual(ProcessIdentity.query(pid), .absent)
        withExtendedLifetime(instance) {}
    }
}
