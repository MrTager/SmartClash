import XCTest
@testable import ProxyLogic

final class NodeLatencyTests: XCTestCase {
    private func nodes(source: Source = Source(name: "A", url: "https://example.com")) throws -> [ProxyNode] {
        try SubscriptionParser.parse("""
        proxies:
          - {name: A, type: ss, server: a.example.com, port: 443, cipher: aes-128-gcm, password: test}
          - {name: B, type: ss, server: b.example.com, port: 443, cipher: aes-128-gcm, password: test}
          - {name: C, type: ss, server: c.example.com, port: 443, cipher: aes-128-gcm, password: test}
          - {name: D, type: ss, server: d.example.com, port: 443, cipher: aes-128-gcm, password: test}
        """, source: source)
    }
    func testLatencySortPlacesSuccessBeforeUntestedAndFailure() throws {
        let pool = try nodes()
        let samples = [pool[0].id: NodeLatency(milliseconds: nil, failure: "timeout"),
                       pool[1].id: NodeLatency(milliseconds: 900), pool[3].id: NodeLatency(milliseconds: 120)]
        XCTAssertEqual(NodeRanking.sorted(pool, latencies: samples).map(\.id), [pool[3], pool[1], pool[2], pool[0]].map(\.id))
        XCTAssertEqual(NodeRanking.sorted(pool, latencies: samples, order: .original).map(\.id), pool.map(\.id))
    }
    func testCandidatesInterleaveSourcesAndCoverEveryNode() throws {
        let a = Source(name: "A", url: "https://example.com"), b = Source(name: "B", url: "https://example.org")
        let raw = try nodes(source: a)
        // Distinct credentials keep these nodes separate across sources.
        let other = try SubscriptionParser.parse("""
        proxies:
          - {name: E, type: trojan, server: e.example.com, port: 443, password: test}
          - {name: F, type: trojan, server: f.example.com, port: 443, password: test}
        """, source: b)
        let ranked = NodeRanking.connectionCandidates(raw + other, latencies: [:])
        XCTAssertEqual(Array(ranked.prefix(4)).map(\.sourceID), [a.id, b.id, a.id, b.id])
        XCTAssertEqual(Set(ranked.map(\.id)), Set((raw + other).map(\.id)))
    }
    func testOnlyFreshKnownSuccessesArePromoted() throws {
        let pool = try nodes(); let now = Date()
        let samples = [pool[1].id: NodeLatency(milliseconds: 10, checked: now.addingTimeInterval(-301)),
                       pool[2].id: NodeLatency(milliseconds: 100, checked: now.addingTimeInterval(-10))]
        XCTAssertEqual(NodeRanking.connectionCandidates(pool, latencies: samples, now: now).first?.id, pool[2].id)
        XCTAssertFalse(NodeLatency(milliseconds: 1, checked: now.addingTimeInterval(10)).isFresh(now: now))
    }
    private func regionNode(_ id: String, _ label: String, source: UUID = UUID()) -> ProxyNode {
        ProxyNode(id: id, name: label, sourceID: source,
            payload: ["name": label, "type": "ss", "server": "\(id).example.com", "port": 443,
                "cipher": "aes-128-gcm", "password": "fixture"])
    }
    func testRecognizesHongKongLabelsWithoutMatchingOtherWords() {
        for label in ["中国香港 01", "HK01", "[hK] 02", "HKG-03", "Hong Kong 04", "Hong-Kong",
                      "HongKong", "\u{1f1ed}\u{1f1f0} 05", "专线 · HK 06"] {
            XCTAssertTrue(NodeRanking.isHongKong(regionNode("n", label)), label)
        }
        for label in ["Tokyo", "Singapore", "US 01", "SHK-01", "HKT 02", "HKEX", "日本 · abcd12345678"] {
            XCTAssertFalse(NodeRanking.isHongKong(regionNode("n", label)), label)
        }
    }
    func testRegionPreferenceIsStableAndRetainsFallbacks() {
        let pool = [regionNode("hk1", "HK01"), regionNode("jp", "JP01"),
                    regionNode("hk2", "中国香港02"), regionNode("sg", "SG01")]
        XCTAssertEqual(NodeRanking.preferNonHongKong(pool).map(\.id), ["jp", "sg", "hk1", "hk2"])
        XCTAssertEqual(NodeRanking.preferNonHongKong([pool[0]]).map(\.id), ["hk1"])
        XCTAssertTrue(NodeRanking.preferNonHongKong([]).isEmpty)
    }
    func testConnectionPreferenceOutranksFreshHongKongLatencyButNotListSort() {
        let pool = [regionNode("hk", "HK01"), regionNode("jp", "JP01")]
        let now = Date()
        let samples = ["hk": NodeLatency(milliseconds: 5, checked: now)]
        XCTAssertEqual(NodeRanking.connectionCandidates(pool, latencies: samples, now: now).map(\.id), ["jp", "hk"])
        XCTAssertEqual(NodeRanking.sorted(pool, latencies: samples).map(\.id), ["hk", "jp"])
    }
    func testSourceRotationIsPreservedWithinRegionTiers() {
        let a = UUID(), b = UUID()
        let pool = [regionNode("hk1", "HK01", source: a), regionNode("jp", "JP01", source: a),
                    regionNode("us", "US01", source: a), regionNode("hk2", "HK02", source: b),
                    regionNode("sg", "SG01", source: b)]
        XCTAssertEqual(NodeRanking.connectionCandidates(pool, latencies: [:]).map(\.id), ["jp", "sg", "us", "hk1", "hk2"])
    }
    func testRegionPreferenceUsesChainExitRatherThanEntry() throws {
        let pool = [regionNode("hk", "HK01"), regionNode("jp", "JP01")]
        var options = ChainOptions(); options.enabled = true
        options.entryIDs = ["hk"]; options.exitIDs = ["jp"]
        XCTAssertFalse(NodeRanking.isHongKong(try XCTUnwrap(ChainBuilder.routes(nodes: pool, options: options).first)))
        options.entryIDs = ["jp"]; options.exitIDs = ["hk"]
        XCTAssertTrue(NodeRanking.isHongKong(try XCTUnwrap(ChainBuilder.routes(nodes: pool, options: options).first)))
    }
    func testAnonymousAutoChainRetainsOriginalExitRegion() throws {
        let pool = [regionNode("hk", "HK01"), regionNode("jp", "JP01")]
        let hkPlan = try DynamicRouter.plan(nodes: pool, round: 0, budget: 2)
        let jpPlan = try DynamicRouter.plan(nodes: pool, round: 1, budget: 2)
        XCTAssertTrue(NodeRanking.isHongKong(try XCTUnwrap(hkPlan.candidates.first { $0.name.hasPrefix("AUTO-") })))
        XCTAssertFalse(NodeRanking.isHongKong(try XCTUnwrap(jpPlan.candidates.first { $0.name.hasPrefix("AUTO-") })))
    }
    func testManualPreferredHongKongNodeIsNotOverridden() throws {
        let pool = [regionNode("hk", "HK01"), regionNode("jp", "JP01")]
        let data = try Configuration.make(nodes: pool, secret: "fixture", tun: false, preferredName: pool[0].name)
        let config = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let groups = try XCTUnwrap(config["proxy-groups"] as? [[String: Any]])
        XCTAssertEqual((groups.first?["proxies"] as? [String])?.first, "HK01")
        let payloads = try XCTUnwrap(config["proxies"] as? [[String: Any]])
        XCTAssertTrue(payloads.allSatisfy { $0["exitName"] == nil && $0["exit_name"] == nil })
    }
    func testDelayPathEscapesTheEntireNodeNameAsOneSegment() throws {
        let name = "香港 / A?#% 中文"
        let path = DelayEndpoint.path(name: name)
        let url = try XCTUnwrap(URL(string: "http://127.0.0.1:\(Configuration.controllerPort)" + path))
        let parts = try XCTUnwrap(URLComponents(url: url, resolvingAgainstBaseURL: false))
        let segments = parts.percentEncodedPath.split(separator: "/")
        XCTAssertEqual(segments.count, 3)
        XCTAssertEqual(String(segments[1]).removingPercentEncoding, name)
        XCTAssertEqual(parts.queryItems?.first(where: { $0.name == "url" })?.value, DelayEndpoint.target)
        XCTAssertEqual(parts.queryItems?.first(where: { $0.name == "expected" })?.value, "204")
    }
    func testGenericInternetAndOpenAIHealthAreIndependent() {
        XCTAssertEqual(InternetPolicy.classify(code: 204, elapsed: 20).health, .healthy)
        for code in [0, 200, 302, 403, 500] {
            XCTAssertEqual(InternetPolicy.classify(code: code, elapsed: 50).health, .offline)
        }
    }
    func testSingleHopModeDoesNotBuildUntestedChains() throws {
        let pool = try nodes()
        let first = try DynamicRouter.plan(nodes: pool, round: 0, budget: 2, allowChains: false)
        let second = try DynamicRouter.plan(nodes: pool, round: 1, budget: 2, allowChains: false)
        XCTAssertTrue(first.definitions.isEmpty)
        XCTAssertEqual(Set((first.candidates + second.candidates).map(\.id)), Set(pool.map(\.id)))
        XCTAssertTrue(first.candidates.allSatisfy { $0.payload["dialer-proxy"] == nil })
    }
    func testTunPinsPhysicalInterfaceAndStartsWithVerifiedNode() throws {
        let pool = try nodes()
        let data = try Configuration.make(nodes: pool, secret: "test", tun: true, outboundInterface: "en11", preferredName: pool[2].name)
        let document = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(document["interface-name"] as? String, "en11")
        XCTAssertEqual((document["tun"] as? [String: Any])?["auto-detect-interface"] as? Bool, false)
        let groups = try XCTUnwrap(document["proxy-groups"] as? [[String: Any]])
        XCTAssertEqual((groups.first?["proxies"] as? [String])?.first, pool[2].name)
        XCTAssertNotNil((document["dns"] as? [String: Any])?["proxy-server-nameserver"])
    }
}
