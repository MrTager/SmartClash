import XCTest
@testable import ProxyLogic

final class ProxyLogicTests: XCTestCase {
    private let source = Source(name: "测试订阅", url: "https://example.com/sub")
    private let fixture = """
    mixed-port: 9999
    external-controller: 0.0.0.0:9999
    secret: unsafe
    proxies:
      - name: Test
        type: ss
        server: example.com
        port: 443
        cipher: aes-128-gcm
        password: test-password
    rules: [MATCH,DIRECT]
    """
    func testMergeDeduplicatesAcrossNamesAndSources() throws {
        let a = try SubscriptionParser.parse(fixture, source: source)
        let b = try SubscriptionParser.parse(fixture.replacingOccurrences(of: "name: Test", with: "name: Renamed"), source: Source(name: "B", url: "https://example.org"))
        XCTAssertEqual(SubscriptionParser.merge([a, b]).count, 1)
        XCTAssertEqual(a.first?.id, b.first?.id)
    }
    func testDifferentCredentialsRemainSeparate() throws {
        let a = try SubscriptionParser.parse(fixture, source: source)
        let b = try SubscriptionParser.parse(fixture.replacingOccurrences(of: "test-password", with: "different-password"), source: source)
        XCTAssertEqual(SubscriptionParser.merge([a, b]).count, 2)
    }
    func testRemoteConfigCannotOverrideControllerOrRules() throws {
        let nodes = try SubscriptionParser.parse(fixture, source: source)
        let data = try Configuration.make(nodes: nodes, secret: "local-secret", tun: false)
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(json["external-controller"] as? String, "127.0.0.1:\(Configuration.controllerPort)")
        XCTAssertEqual(json["secret"] as? String, "local-secret")
        XCTAssertEqual(json["allow-lan"] as? Bool, false)
        XCTAssertEqual((json["rules"] as? [String])?.last, "MATCH,SMART")
        let listeners = try XCTUnwrap(json["listeners"] as? [[String: Any]])
        XCTAssertEqual(listeners.first?["proxy"] as? String, "PROBE")
        XCTAssertEqual(listeners.first?["port"] as? Int, Configuration.probePort)
    }
    func testExportHasNodesButNoLocalControllerSecret() throws {
        let nodes = try SubscriptionParser.parse(fixture, source: source)
        let data = try Configuration.export(nodes: nodes)
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertNil(json["external-controller"]); XCTAssertNil(json["secret"])
        XCTAssertEqual((json["proxies"] as? [[String: Any]])?.count, 1)
        XCTAssertEqual(try SubscriptionParser.parse(String(decoding: data, as: UTF8.self), source: source).count, 1)
    }
    func testRejectsLocalCredentialFileAndProxyChains() {
        XCTAssertThrowsError(try SubscriptionParser.parse(fixture.replacingOccurrences(of: "password: test-password", with: "password: test-password\n    private-key: /tmp/private.pem"), source: source))
        XCTAssertThrowsError(try SubscriptionParser.parse(fixture.replacingOccurrences(of: "password: test-password", with: "password: test-password\n    dialer-proxy: Other"), source: source))
    }
    func testPreservesWebSocketTransportPath() throws {
        let input = """
        proxies:
          - name: ws
            type: vmess
            server: example.com
            port: 443
            uuid: 00000000-0000-0000-0000-000000000001
            alterId: 0
            cipher: auto
            network: ws
            ws-opts: {path: /api/ws}
        """
        XCTAssertEqual(try SubscriptionParser.parse(input, source: source).count, 1)
    }
    func testRejectsEmptyAndUnsupportedSubscriptions() {
        XCTAssertThrowsError(try SubscriptionParser.parse("proxies: []", source: source))
        XCTAssertThrowsError(try SubscriptionParser.parse("vmess://abcdef", source: source))
        XCTAssertThrowsError(try SubscriptionParser.parse(fixture.replacingOccurrences(of: "type: ss", with: "type: unknown"), source: source))
    }
    func testLegacyWebSocketFieldsNormalizeAndDeduplicate() throws {
        let legacy = """
        proxies:
          - name: WS
            type: vmess
            server: example.com
            port: 443
            uuid: 00000000-0000-0000-0000-000000000001
            network: ws
            ws-path: /legacy
            ws-headers: {Host: example.com}
        """
        let nodes = try SubscriptionParser.parse(legacy, source: source)
        let options = try XCTUnwrap(nodes[0].payload["ws-opts"] as? [String: Any])
        XCTAssertEqual(options["path"] as? String, "/legacy")
        XCTAssertNil(nodes[0].payload["ws-path"])
        XCTAssertNil(nodes[0].payload["ws-headers"])
        let modern = legacy.replacingOccurrences(of: "ws-path: /legacy\n    ws-headers: {Host: example.com}",
            with: "ws-opts: {path: /legacy, headers: {Host: example.com}}")
        let current = try SubscriptionParser.parse(modern, source: source)
        XCTAssertEqual(nodes[0].id, current[0].id)
    }
    func testBuiltInObfsIsAcceptedButExternalPluginsRemainRejected() throws {
        let obfs = fixture.replacingOccurrences(of: "password: test-password",
            with: "password: test-password\n    plugin: obfs\n    plugin-opts: {mode: tls, host: example.com}")
        XCTAssertEqual(try SubscriptionParser.parse(obfs, source: source).first?.payload["plugin"] as? String, "obfs")
        XCTAssertThrowsError(try SubscriptionParser.parse(obfs.replacingOccurrences(of: "plugin: obfs", with: "plugin: /tmp/run"), source: source))
        XCTAssertThrowsError(try SubscriptionParser.parse(obfs.replacingOccurrences(of: "host: example.com", with: "path: /tmp/run"), source: source))
    }
    func testParserErrorsDoNotLeakCredentials() {
        do { _ = try SubscriptionParser.parse("password: [TOP-SECRET", source: source); XCTFail("Expected failure") }
        catch { XCTAssertFalse(error.localizedDescription.contains("TOP-SECRET")) }
    }
    func testOpenAIAuthenticationResponseProvesReachability() {
        let body = "{\"error\":{\"message\":\"You didn't provide an API key.\",\"type\":\"invalid_request_error\"}}"
        XCTAssertEqual(HealthPolicy.classify(code: 401, body: body, elapsed: 100).health, .healthy)
        XCTAssertEqual(HealthPolicy.classify(code: 401, body: "<html>proxy login</html>", elapsed: 100).health, .unknown)
    }
    func testBlockedLimitedAndChallengeAreDistinct() {
        XCTAssertEqual(HealthPolicy.classify(code: 403, body: "unsupported_country_region_territory", elapsed: 100).health, .blocked)
        XCTAssertEqual(HealthPolicy.classify(code: 429, body: "rate limit", elapsed: 100).health, .limited)
        XCTAssertEqual(HealthPolicy.classify(code: 403, body: "Cloudflare challenge", elapsed: 100).health, .unknown)
        XCTAssertEqual(HealthPolicy.classify(code: 200, body: "<html>challenge</html>", elapsed: 100).health, .unknown)
        XCTAssertEqual(HealthPolicy.classify(code: 200, body: "{\"object\":\"list\",\"data\":[]}", elapsed: 100).health, .healthy)
    }
    func testNetworkFailuresAndServerErrors() {
        XCTAssertEqual(HealthPolicy.classify(code: 0, body: "", elapsed: 8000).health, .offline)
        XCTAssertEqual(HealthPolicy.classify(code: 503, body: "", elapsed: 50).health, .offline)
    }
    func testFailureThresholdAndCooldown() {
        let now = Date()
        XCTAssertFalse(HealthPolicy.shouldFailover(failures: 1, lastSwitch: .distantPast, now: now))
        XCTAssertFalse(HealthPolicy.shouldFailover(failures: 2, lastSwitch: now.addingTimeInterval(-29), now: now))
        XCTAssertTrue(HealthPolicy.shouldFailover(failures: 2, lastSwitch: now.addingTimeInterval(-31), now: now))
    }
    func testEmptyRuntimeConfigurationRejected() {
        XCTAssertThrowsError(try Configuration.make(nodes: [], secret: "test", tun: false))
    }
    func testSerializedConfigurationDoesNotUseJSONOnlySlashEscapes() throws {
        let nodes = try SubscriptionParser.parse(fixture.replacingOccurrences(of: "name: Test", with: "name: Test/Slash"), source: source)
        for data in [try Configuration.make(nodes: nodes, secret: "test", tun: false), try Configuration.export(nodes: nodes)] {
            XCTAssertFalse(String(decoding: data, as: UTF8.self).contains("\\/"))
            XCTAssertNotNil(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        }
    }
    func testInformationalNodesAreSkippedAtImport() throws {
        // 订阅商会在 proxies 里塞入「剩余流量 / 到期时间」等信息节点，不是真实代理。
        let text = """
        proxies:
          - {name: "剩余流量：100 GB", type: ss, server: info.example.com, port: 443, cipher: aes-128-gcm, password: x}
          - {name: "套餐到期：2027-01-01", type: ss, server: info.example.com, port: 443, cipher: aes-128-gcm, password: x}
          - {name: 香港 01, type: ss, server: hk.example.com, port: 443, cipher: aes-128-gcm, password: y}
        """
        let nodes = try SubscriptionParser.parse(text, source: source)
        XCTAssertEqual(nodes.count, 1)
        XCTAssertTrue(nodes[0].name.hasPrefix("香港 01"))
        XCTAssertTrue(SubscriptionParser.isInformational("剩余流量：100 GB"))
        XCTAssertTrue(SubscriptionParser.isInformational("官网：example.com"))
        XCTAssertFalse(SubscriptionParser.isInformational("香港 01"))
    }
}
