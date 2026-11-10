import Foundation
public enum Health: String { case healthy, blocked, limited, unknown, offline }
public struct ProbeResult {
    public let health: Health
    public let milliseconds: Int
    public let detail: String
    public init(health: Health, milliseconds: Int, detail: String) {
        self.health = health; self.milliseconds = milliseconds; self.detail = detail
    }
}
public enum HealthPolicy {
    /// 业务链路固定每 10 秒检测一次；动态模式不再用 1 秒轮询主链路。
    public static let monitorInterval: Int = 10
    /// 明确断线后立即切换，但两次切换至少间隔 10 秒，避免节点抖动。
    public static let failoverCooldown: TimeInterval = 10
    public static func recoveryCooldown(failures: Int) -> TimeInterval {
        switch max(0, failures) {
        case 0: return 10
        case 1: return 20
        case 2: return 40
        default: return 60
        }
    }
    public static func classify(code: Int, body: String, elapsed: Int) -> ProbeResult {
        let lower = body.lowercased()
        let health: Health
        if ["unsupported_country", "unsupported country", "region is not supported", "country is not supported"].contains(where: lower.contains) { health = .blocked }
        else if code == 429 { health = .limited }
        else if (200..<300).contains(code), let data = body.data(using: .utf8),
                let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                json["object"] as? String == "list", json["data"] is [Any] { health = .healthy }
        // Without an API key only a structured OpenAI authentication error proves API reachability.
        else if code == 401, let data = body.data(using: .utf8),
                let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let error = json["error"] as? [String: Any],
                let message = error["message"] as? String,
                message.lowercased().contains("api key") || message.lowercased().contains("authentication") { health = .healthy }
        else if code == 0 || code >= 500 { health = .offline }
        else { health = .unknown }
        return ProbeResult(health: health, milliseconds: elapsed, detail: "HTTP \(code) · \(health.rawValue)")
    }
    public static func shouldFailover(
        failures: Int, lastSwitch: Date, now: Date,
        threshold: Int = 1, cooldown: TimeInterval = failoverCooldown
    ) -> Bool {
        failures >= threshold && now.timeIntervalSince(lastSwitch) >= cooldown
    }
}
