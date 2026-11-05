import Foundation
import ProxyLogic

/// Xray 没有 Clash /traffic 接口，通过 StatsService CLI 读取入站累计流量。
enum XrayStats {
    static func totals(executable: URL) async -> (up: UInt64, down: UInt64)? {
        guard FileManager.default.isExecutableFile(atPath: executable.path) else { return nil }
        guard let result = try? await Command.run(executable.path, [
            "api", "statsquery",
            "--server=127.0.0.1:\(KernelConfiguration.xrayAPIPort)",
            "-pattern", ""
        ]), result.code == 0 else { return nil }
        guard let start = result.text.firstIndex(of: "{"),
              let end = result.text.lastIndex(of: "}"),
              start <= end else { return nil }
        let json = String(result.text[start...end])
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let stats = object["stat"] as? [[String: Any]] else { return nil }
        var up: UInt64 = 0
        var down: UInt64 = 0
        for item in stats {
            guard let name = item["name"] as? String, name.hasPrefix("inbound>>>") else { continue }
            let value = (item["value"] as? NSNumber)?.uint64Value
                ?? UInt64(item["value"] as? String ?? "")
                ?? 0
            if name.hasSuffix(">>>traffic>>>uplink") { up += value }
            else if name.hasSuffix(">>>traffic>>>downlink") { down += value }
        }
        return (up, down)
    }
}
