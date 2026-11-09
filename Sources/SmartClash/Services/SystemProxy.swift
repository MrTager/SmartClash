import Foundation
import ProxyLogic

@MainActor
final class SystemProxy {
    private let legacyBackup = Files.root.appendingPathComponent("proxy-backup.json")
    var hasLegacyBackup: Bool { FileManager.default.fileExists(atPath: legacyBackup.path) }
    var hasBackup: Bool { PrivilegedClient.shared.proxyPending || hasLegacyBackup }
    private var terminating = false

    func closeGate() { terminating = true }
    func enable() async throws {
        guard !terminating else { throw CancellationError() }
        try Task.checkCancellation()
        guard !hasBackup else { throw ProxyError.message("已有代理恢复记录，请先恢复旧设置") }
        // 快照、持久化与字段级恢复均由同一后台会话完成，GUI 不再保存网络快照。
        do { try await PrivilegedClient.shared.setSystemProxy(enabled: true) }
        catch { try? await restore(); throw error }
        guard !terminating else { throw CancellationError() }
        try Task.checkCancellation()
    }
    func restore() async throws {
        var failures: [String] = []
        do { try await PrivilegedClient.shared.setSystemProxy(enabled: false) }
        catch { failures.append(error.localizedDescription) }
        if hasLegacyBackup {
            failures.append("发现旧版 proxy-backup.json；无法证明当前网络字段归属，记录已保留，请核实并手动恢复旧设置")
        }
        if !failures.isEmpty { throw ProxyError.message(failures.joined(separator: "；")) }
    }
}
