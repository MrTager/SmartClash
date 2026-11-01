import Foundation
public enum StandbyPolicy {
    public static func usable(_ result: ProbeResult, checked: Date, now: Date, ttl: TimeInterval = 15) -> Bool {
        result.health == .healthy && now.timeIntervalSince(checked) >= 0 && now.timeIntervalSince(checked) <= ttl
    }
}
