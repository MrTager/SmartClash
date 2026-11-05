import Foundation

/// Policy for the egress check performed immediately after a TUN takeover.
///
/// A freshly booted privileged core needs time to install the default route, start
/// DNS hijacking and establish the first proxy connection. A single 6-second probe
/// used to roll back a perfectly healthy TUN on any transient exit-node hiccup,
/// which made TUN impossible to enable on unstable links. Kept as pure logic so the
/// retry window and the failure message are covered by unit tests.
public enum TunVerificationPolicy {
    /// Number of retries after the first probe (mirrors the Windows client).
    public static let maxRetries = 5
    /// Delay between two probes, giving the route/DNS time to converge.
    public static let retryDelay: TimeInterval = 2
    /// Hard ceiling for the whole verification phase.
    public static let deadline: TimeInterval = 40

    /// Whether retry number `retry` (1-based) should run after `elapsed` seconds.
    public static func shouldRetry(_ retry: Int, elapsed: TimeInterval) -> Bool {
        guard retry >= 1, retry <= maxRetries, elapsed >= 0 else { return false }
        return elapsed + retryDelay < deadline
    }

    public static func progress(_ retry: Int) -> String {
        "TUN 已接管，正在验证出网（\(retry)/\(maxRetries)）"
    }

    public static func failureMessage(attempts: Int, last: String) -> String {
        "TUN 接管后连续 \(attempts) 次出网验证未通过（最后一次：\(last)）；请检查出口节点状态、DNS、物理网卡或其他 VPN/TUN 冲突"
    }

    public static func recoveryMessage(_ reason: String, stopError: String) -> String {
        "\(reason)；自动恢复未完成（\(stopError)），TUN 可能仍接管系统流量，请点击“恢复网络”并允许系统授权"
    }
}
