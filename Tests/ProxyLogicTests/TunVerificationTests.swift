import XCTest
import ProxyLogic

final class TunVerificationTests: XCTestCase {
    func testRetryWindowIsBounded() {
        XCTAssertFalse(TunVerificationPolicy.shouldRetry(0, elapsed: 0))
        XCTAssertTrue(TunVerificationPolicy.shouldRetry(1, elapsed: 0))
        XCTAssertTrue(TunVerificationPolicy.shouldRetry(5, elapsed: 10))
        XCTAssertFalse(TunVerificationPolicy.shouldRetry(6, elapsed: 0))
        XCTAssertFalse(TunVerificationPolicy.shouldRetry(1, elapsed: 39))
        XCTAssertFalse(TunVerificationPolicy.shouldRetry(1, elapsed: -1))
    }

    func testProgressShowsAttemptNumber() {
        XCTAssertEqual(TunVerificationPolicy.progress(2), "TUN 已接管，正在验证出网（2/5）")
    }

    func testFailureMessageCarriesProbeDiagnostics() {
        let message = TunVerificationPolicy.failureMessage(attempts: 6,
            last: "HTTPS 出网未通过（HTTP 0，curl: (6) Could not resolve host）")
        XCTAssertTrue(message.contains("连续 6 次"))
        XCTAssertTrue(message.contains("Could not resolve host"))
        XCTAssertTrue(message.contains("出口节点状态"))
        XCTAssertFalse(message.contains("已自动恢复网络"))
    }

    func testRecoveryMessageReportsUnfinishedRollback() {
        let message = TunVerificationPolicy.recoveryMessage("TUN 接管失败", stopError: "后台服务连接中断")
        XCTAssertTrue(message.contains("自动恢复未完成"))
        XCTAssertTrue(message.contains("可能仍接管系统流量"))
        XCTAssertTrue(message.contains("恢复网络"))
    }

    func testInternetProbeDiagnosticsArePreserved() {
        let plain = InternetPolicy.classify(code: 0, elapsed: 120)
        XCTAssertEqual(plain.health, .offline)
        XCTAssertEqual(plain.detail, "HTTPS 出网未通过（HTTP 0）")

        let detailed = InternetPolicy.classify(code: 0, elapsed: 120,
            transportError: "curl: (6) Could not resolve host")
        XCTAssertEqual(detailed.health, .offline)
        XCTAssertTrue(detailed.detail.contains("curl: (6)"))

        let healthy = InternetPolicy.classify(code: 204, elapsed: 88, transportError: "ignored")
        XCTAssertEqual(healthy.health, .healthy)
        XCTAssertEqual(healthy.detail, "HTTPS 出网正常")
        XCTAssertEqual(healthy.milliseconds, 88)
    }
}
