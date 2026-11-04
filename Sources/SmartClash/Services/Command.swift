import Foundation
import ProxyLogic
struct CommandResult { let code: Int32; let text: String }
private final class CancellableCommand: @unchecked Sendable {
    private let lock = NSLock()
    private var process: Process?
    private var cancelled = false
    func launch(_ process: Process) throws {
        lock.lock(); defer { lock.unlock() }
        guard !cancelled else { throw CancellationError() }
        try process.run(); self.process = process
    }
    func cancel() {
        lock.lock(); cancelled = true
        let running = process
        lock.unlock()
        if let running, running.isRunning { running.terminate() }
    }
}
enum Command {
    static func run(_ executable: String, _ args: [String], cancellable: Bool = false) async throws -> CommandResult {
        let command = CancellableCommand()
        return try await withTaskCancellationHandler {
            if cancellable { try Task.checkCancellation() }
            let result = try await Task.detached(priority: .utility) {
            let p = Process(); p.executableURL = URL(fileURLWithPath: executable); p.arguments = args
            let pipe = Pipe(); p.standardOutput = pipe; p.standardError = pipe
            var environment = ProcessInfo.processInfo.environment
            environment["LC_ALL"] = "C"; p.environment = environment
            try command.launch(p)
            let bytes = pipe.fileHandleForReading.readDataToEndOfFile()
            p.waitUntilExit()
            return CommandResult(code: p.terminationStatus, text: String(decoding: bytes, as: UTF8.self))
            }.value
            if cancellable { try Task.checkCancellation() }
            return result
        } onCancel: {
            if cancellable { command.cancel() }
        }
    }
    static func quote(_ text: String) -> String { "'" + text.replacingOccurrences(of: "'", with: "'\\''") + "'" }
    static func admin(_ script: String) async throws {
        // JSON string encoding is suitable for the AppleScript string literal, not shell quoting.
        let encoder = JSONEncoder(); encoder.outputFormatting = [.withoutEscapingSlashes]
        let literal = String(data: try encoder.encode(script), encoding: .utf8)!
        let r = try await run("/usr/bin/osascript", ["-e", "do shell script \(literal) with administrator privileges"])
        guard r.code == 0 else { throw ProxyError.message("管理员操作未完成（可能取消授权）。\(r.text.prefix(300))") }
    }
}
enum Files {
    static let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("SmartClash")
    static func setup() throws {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
                                               attributes: [.posixPermissions: 0o700])
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: root.path)
    }
    static func write(_ data: Data, to url: URL) throws {
        try data.write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }
}
