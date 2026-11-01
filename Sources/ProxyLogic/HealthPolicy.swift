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
    public static func shouldFailover(failures: Int, lastSwitch: Date, now: Date, threshold: Int = 2) -> Bool {
        failures >= threshold && now.timeIntervalSince(lastSwitch) >= 30
    }
}
