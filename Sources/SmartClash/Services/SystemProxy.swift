import Foundation
import ProxyLogic
struct ProxySnapshot: Codable {
    var service: String
    var kind: String
    var host: String
    var port: Int
    var enabled: Bool
}
@MainActor
final class SystemProxy {
    private let backup = Files.root.appendingPathComponent("proxy-backup.json")
    var hasBackup: Bool { FileManager.default.fileExists(atPath: backup.path) }
    private let kinds = ["webproxy", "securewebproxy", "socksfirewallproxy"]
    func enable() async throws {
        guard !hasBackup else { throw ProxyError.message("已有代理恢复记录，请先恢复旧设置") }
        let all = try await Command.run("/usr/sbin/networksetup", ["-listallnetworkservices"])
        guard all.code == 0 else { throw ProxyError.message("无法列出系统网络服务") }
        let services = all.text.components(separatedBy: .newlines).dropFirst().filter { !$0.isEmpty && !$0.hasPrefix("*") }
        var snapshots: [ProxySnapshot] = []
        for service in services {
            let pac = try await Command.run("/usr/sbin/networksetup", ["-getautoproxyurl", service])
            let discovery = try await Command.run("/usr/sbin/networksetup", ["-getproxyautodiscovery", service])
            guard pac.code == 0, discovery.code == 0 else { throw ProxyError.message("无法检查网络服务的自动代理设置") }
            guard !pac.text.contains("Enabled: Yes"), !discovery.text.contains(": On") else {
                throw ProxyError.message("服务 \(service) 已启用 PAC / 自动代理发现，请先在系统设置关闭，避免代理策略冲突")
            }
            for kind in kinds {
                let r = try await Command.run("/usr/sbin/networksetup", ["-get" + kind, service])
                guard r.code == 0 else { throw ProxyError.message("无法读取网络服务代理：\(service)") }
                var values: [String: String] = [:]
                for line in r.text.components(separatedBy: .newlines) {
                    let pair = line.split(separator: ":", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
                    if pair.count == 2 { values[pair[0]] = pair[1] }
                }
                guard values["Authenticated Proxy Enabled"] != "1" else {
                    throw ProxyError.message("服务 \(service) 使用认证代理，无法完整恢复其密码。请手动关闭该认证代理后重试")
                }
                snapshots.append(ProxySnapshot(service: service, kind: kind, host: values["Server"] ?? "", port: Int(values["Port"] ?? "") ?? 0, enabled: values["Enabled"] == "Yes"))
            }
        }
        guard !snapshots.isEmpty else { throw ProxyError.message("没有可用网络服务") }
        // Persist BEFORE changing any service, so a partial change or crash remains recoverable.
        try Files.write(JSONEncoder().encode(snapshots), to: backup)
        var commands = ["set -e"]
        for item in snapshots {
            commands.append(command("-set" + item.kind, [item.service, "127.0.0.1", String(Configuration.proxyPort)]))
            commands.append(command("-set" + item.kind + "state", [item.service, "on"]))
        }
        do { try await Command.admin(commands.joined(separator: "\n")) }
        catch { try? await restore(); throw error }
    }
    func restore() async throws {
        guard hasBackup else { return }
        let snapshots = try JSONDecoder().decode([ProxySnapshot].self, from: Data(contentsOf: backup))
        var commands = ["set -e"]
        for item in snapshots {
            if !item.host.isEmpty && item.port > 0 {
                commands.append(command("-set" + item.kind, [item.service, item.host, String(item.port)]))
            }
            commands.append(command("-set" + item.kind + "state", [item.service, item.enabled ? "on" : "off"]))
        }
        try await Command.admin(commands.joined(separator: "\n"))
        try FileManager.default.removeItem(at: backup)
    }
    private func command(_ op: String, _ args: [String]) -> String {
        (["/usr/sbin/networksetup", op] + args).map(Command.quote).joined(separator: " ")
    }
}
