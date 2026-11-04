import Foundation
import ProxyLogic

enum NetworkEnvironment {
    static func parseDefaultInterface(_ output: String) -> String? {
        let interface = output.components(separatedBy: .newlines).compactMap { line -> String? in
            let pair = line.split(separator: ":", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            return pair.count == 2 && pair[0] == "interface" ? pair[1] : nil
        }.first
        guard let interface, interface.range(of: "^[A-Za-z][A-Za-z0-9]{0,20}$", options: .regularExpression) != nil else { return nil }
        return interface
    }
    static func defaultInterface(forTun: Bool) async throws -> String? {
        let result = try await Command.run("/sbin/route", ["-n", "get", "default"])
        guard result.code == 0, let interface = parseDefaultInterface(result.text) else {
            if forTun { throw ProxyError.message("无法确认物理网络出口，未启用 TUN；请检查本机网络") }
            return nil
        }
        guard !interface.hasPrefix("utun"), !interface.hasPrefix("tun"), interface != "lo0" else {
            if forTun { throw ProxyError.message("默认路由已由其他 VPN 接管，请先关闭其 TUN，避免双重路由导致断网") }
            return nil
        }
        return interface
    }
}
