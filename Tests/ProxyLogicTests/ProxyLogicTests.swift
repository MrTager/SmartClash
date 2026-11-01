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
        XCTAssertEqual(json["external-controller"] as? String, "127.0.0.1:19090")
        XCTAssertEqual(json["secret"] as? String, "local-secret")
        XCTAssertEqual(json["allow-lan"] as? Bool, false)
        XCTAssertEqual((json["rules"] as? [String])?.last, "MATCH,SMART")
        let listeners = try XCTUnwrap(json["listeners"] as? [[String: Any]])
        XCTAssertEqual(listeners.first?["proxy"] as? String, "PROBE")
        XCTAssertEqual(listeners.first?["port"] as? Int, 17891)
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
}
