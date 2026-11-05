import XCTest
import ProxyLogic
@testable import SmartClash

final class ProxyPortTests: XCTestCase {
    func testValidateProxyPortAcceptsUsablePorts() {
        XCTAssertNil(Configuration.validateProxyPort(7890))
        XCTAssertNil(Configuration.validateProxyPort(8080))
        XCTAssertNil(Configuration.validateProxyPort(65535))
    }
    func testValidateProxyPortRejectsReservedAndInvalid() {
        // 低于 1024 属于系统保留段。
        XCTAssertNotNil(Configuration.validateProxyPort(80))
        XCTAssertNotNil(Configuration.validateProxyPort(1023))
        XCTAssertNotNil(Configuration.validateProxyPort(65536))
        // 内部保留端口：探测 / 聚合 / 控制 / Xray。
        XCTAssertNotNil(Configuration.validateProxyPort(Configuration.probePort))
        XCTAssertNotNil(Configuration.validateProxyPort(Configuration.aggregatePort))
        XCTAssertNotNil(Configuration.validateProxyPort(Configuration.controllerPort))
        XCTAssertNotNil(Configuration.validateProxyPort(KernelConfiguration.xraySocksPort))
        XCTAssertNotNil(Configuration.validateProxyPort(KernelConfiguration.xrayAPIPort))
    }
    func testLegacyPreferencesWithoutProxyPortFallBackToDefault() throws {
        // 旧版 preferences.json（无 proxyPort 字段）解码为 nil，
        // AppStore 启动时把 nil 统一回退为默认端口。
        // 旧版偏好文件包含全部基础键，但没有 proxyPort 字段。
        let json = """
        {"sources":[],"interval":60,"automatic":true,"secret":"s","subscriptionToken":"t"}
        """
        let legacy = try JSONDecoder().decode(Preferences.self, from: Data(json.utf8))
        XCTAssertNil(legacy.proxyPort)
        XCTAssertNil(Configuration.validateProxyPort(legacy.proxyPort ?? Configuration.defaultProxyPort))
    }
    func testRuntimeProxyPortFollowsConfiguration() {
        let original = Configuration.proxyPort
        defer { Configuration.proxyPort = original }
        Configuration.proxyPort = 18080
        XCTAssertEqual(Configuration.proxyPort, 18080)
        let generated = try? Configuration.make(nodes: [], secret: "s", tun: false, trafficMode: .direct)
        XCTAssertNotNil(generated)
    }
}
