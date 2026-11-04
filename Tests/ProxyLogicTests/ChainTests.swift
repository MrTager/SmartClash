import XCTest
@testable import ProxyLogic
final class ChainTests: XCTestCase {
    private func nodes() throws -> [ProxyNode] {
        try SubscriptionParser.parse("""
        proxies:
          - {name: HK, type: ss, server: hk.example.com, port: 443, cipher: aes-128-gcm, password: example}
          - {name: US, type: trojan, server: us.example.com, port: 443, password: example}
          - {name: US2, type: trojan, server: us2.example.com, port: 443, password: example}
        """, source: Source(name: "test", url: "https://example.com"))
    }
    func testExitDialsThroughEntry() throws {
        let raw = try nodes(); var options = ChainOptions(); options.enabled = true
        options.entryIDs = [raw[0].id]; options.exitIDs = [raw[1].id, raw[2].id]
        let routes = try ChainBuilder.routes(nodes: raw, options: options)
        XCTAssertEqual(routes.count, 2)
        XCTAssertEqual(routes[0].payload["server"] as? String, "us.example.com")
        XCTAssertEqual(routes[0].payload["dialer-proxy"] as? String, raw[0].name)
        XCTAssertNil(raw[0].payload["dialer-proxy"])
        let data = try Configuration.make(nodes: raw, secret: "secret", tun: false, routes: routes)
        let doc = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let groups = try XCTUnwrap(doc["proxy-groups"] as? [[String: Any]])
        XCTAssertEqual(groups[0]["proxies"] as? [String], routes.map(\.name))
        XCTAssertEqual(groups[1]["proxies"] as? [String], routes.map(\.name) + raw.map(\.name))
        XCTAssertEqual((doc["proxies"] as? [[String: Any]])?.count, 5)
    }
    func testNoSelfLoop() throws {
        let raw = try nodes(); var options = ChainOptions(); options.enabled = true
        options.entryIDs = [raw[0].id]; options.exitIDs = [raw[0].id]
        XCTAssertThrowsError(try ChainBuilder.routes(nodes: raw, options: options))
    }
    func testStaleSelectionDoesNotFallbackToSingleHop() throws {
        var options = ChainOptions(); options.enabled = true
        options.entryIDs = ["deleted"]; options.exitIDs = ["missing"]
        XCTAssertThrowsError(try ChainBuilder.routes(nodes: nodes(), options: options))
    }
    func testDisabledReturnsRawNodes() throws {
        let raw = try nodes()
        XCTAssertEqual(try ChainBuilder.routes(nodes: raw, options: ChainOptions()).map(\.id), raw.map(\.id))
    }
    func testEntryAndExitChangesHaveDifferentIdentity() throws {
        let raw = try nodes(); var options = ChainOptions(); options.enabled = true
        options.entryIDs = [raw[0].id, raw[1].id]; options.exitIDs = [raw[1].id, raw[2].id]
        let routes = try ChainBuilder.routes(nodes: raw, options: options)
        XCTAssertEqual(routes.count, 3)
        XCTAssertEqual(Set(routes.map(\.id)).count, 3)
    }
    func testRejectsUDPOnlyProtocols() throws {
        let raw = try SubscriptionParser.parse("""
        proxies:
          - {name: HK, type: hysteria2, server: hk.example.com, port: 443, password: example}
          - {name: US, type: trojan, server: us.example.com, port: 443, password: example}
        """, source: Source(name: "test", url: "https://example.com"))
        var options = ChainOptions(); options.enabled = true
        options.entryIDs = [raw[0].id]; options.exitIDs = [raw[1].id]
        XCTAssertThrowsError(try ChainBuilder.routes(nodes: raw, options: options))
    }
}

final class DynamicRouterTests: XCTestCase {
    private func pool() throws -> [ProxyNode] {
        let rows = (0..<5).map { "  - {name: N\($0), type: ss, server: n\($0).example.com, port: 443, cipher: aes-128-gcm, password: test}" }.joined(separator: "\n")
        return try SubscriptionParser.parse("proxies:\n" + rows, source: Source(name: "test", url: "https://example.com"))
    }
    func testVariableDepthAndNoCycles() throws {
        let raw = try pool(); let plan = try DynamicRouter.plan(nodes: raw, round: 0)
        let definitions = Dictionary(uniqueKeysWithValues: (raw + plan.definitions).map { ($0.name, $0) })
        var depths = Set<Int>()
        for candidate in plan.candidates {
            var current: String? = candidate.name; var seen = Set<String>(); var servers = Set<String>(); var depth = 0
            while let name = current {
                XCTAssertTrue(seen.insert(name).inserted)
                let node = try XCTUnwrap(definitions[name]); depth += 1
                XCTAssertTrue(servers.insert(node.payload["server"] as! String).inserted)
                current = node.payload["dialer-proxy"] as? String
            }
            depths.insert(depth)
        }
        XCTAssertTrue(depths.contains(1)); XCTAssertTrue(depths.contains(3)); XCTAssertTrue(depths.contains(5))
    }
    func testSearchBudgetAndRotation() throws {
        let raw = try pool()
        let a = try DynamicRouter.plan(nodes: raw, round: 0, budget: 4)
        let b = try DynamicRouter.plan(nodes: raw, round: 1, budget: 4)
        XCTAssertLessThanOrEqual(a.candidates.count, 4)
        XCTAssertNotEqual(a.candidates.map(\.id), b.candidates.map(\.id))
        XCTAssertThrowsError(try DynamicRouter.plan(nodes: [], round: 0))
    }
}

final class StandbyPolicyTests: XCTestCase {
    func testOnlyFreshHealthyResultsAreEligible() {
        let now = Date()
        XCTAssertTrue(StandbyPolicy.usable(ProbeResult(health: .healthy, milliseconds: 10, detail: ""), checked: now.addingTimeInterval(-14), now: now))
        XCTAssertFalse(StandbyPolicy.usable(ProbeResult(health: .healthy, milliseconds: 10, detail: ""), checked: now.addingTimeInterval(-16), now: now))
    }
    func testLimitedFailedAndFutureResultsAreRejected() {
        let now = Date()
        for health in [Health.offline, .limited, .blocked, .unknown] {
            XCTAssertFalse(StandbyPolicy.usable(ProbeResult(health: health, milliseconds: 10, detail: ""), checked: now, now: now))
        }
        XCTAssertFalse(StandbyPolicy.usable(ProbeResult(health: .healthy, milliseconds: 10, detail: ""), checked: now.addingTimeInterval(1), now: now))
    }
}
