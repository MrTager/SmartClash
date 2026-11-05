import XCTest
import Yams
@testable import ProxyLogic

final class TrafficModeTests: XCTestCase {
    private func nodes() throws -> [ProxyNode] {
        try SubscriptionParser.parse("proxies: [{name: fixture, type: trojan, server: example.com, port: 443, password: test}]", source: Source(name: "fixture", url: "https://example.com"))
    }
    func testAllModesPreserveSmartSelectionAndIndependentProbe() throws {
        for mode in TrafficMode.allCases {
            let config = try XCTUnwrap(JSONSerialization.jsonObject(with: Configuration.make(nodes: nodes(), secret: "test", tun: true,
                trafficMode: mode, ruleProviderBaseURL: "http://127.0.0.1:17892/test/rules")) as? [String: Any])
            XCTAssertEqual(config["mode"] as? String, mode.rawValue)
            let groups = try XCTUnwrap(config["proxy-groups"] as? [[String: Any]])
            XCTAssertEqual(groups.first { $0["name"] as? String == "GLOBAL" }?["proxies"] as? [String], ["SMART"])
            let listeners = try XCTUnwrap(config["listeners"] as? [[String: Any]])
            XCTAssertEqual(listeners.first?["proxy"] as? String, "PROBE")
            let providers = try XCTUnwrap(config["rule-providers"] as? [String: [String: Any]])
            XCTAssertEqual(providers.count, 6)
            XCTAssertTrue(providers.values.allSatisfy { $0["interval"] as? Int == 3600 && $0["proxy"] as? String == "DIRECT" })
            let rules = try XCTUnwrap(config["rules"] as? [String])
            XCTAssertLessThan(try XCTUnwrap(rules.firstIndex(of: "RULE-SET,sr-reject-domain,REJECT")), try XCTUnwrap(rules.firstIndex(of: "RULE-SET,sr-proxy-domain,SMART")))
            XCTAssertLessThan(try XCTUnwrap(rules.firstIndex(of: "RULE-SET,sr-proxy-domain,SMART")), try XCTUnwrap(rules.firstIndex(of: "RULE-SET,sr-direct-domain,DIRECT")))
            XCTAssertEqual(rules.last, "MATCH,SMART")
        }
    }
    func testDirectCanStartWithoutAnySubscription() throws {
        let config = try XCTUnwrap(JSONSerialization.jsonObject(with: Configuration.make(nodes: [], secret: "test", tun: false, trafficMode: .direct)) as? [String: Any])
        XCTAssertEqual(config["mode"] as? String, "direct")
        XCTAssertTrue((config["proxies"] as? [[String: Any]])?.isEmpty == true)
        XCTAssertThrowsError(try Configuration.make(nodes: [], secret: "test", tun: false, trafficMode: .global))
    }
    func testConversionPreservesExactSuffixKeywordAndCIDRAndReportsUnsupportedHTTPRules() throws {
        let result = try ShadowrocketRuleParser.parse("""
        #!name=test
        [Rule]
        DOMAIN,EXAMPLE.COM,PROXY
        DOMAIN-SUFFIX,example.org,PROXY
        DOMAIN-SUFFIX,example.org,PROXY
        DOMAIN-KEYWORD,google,PROXY
        IP-CIDR,1.1.1.0/24,PROXY,no-resolve
        IP-CIDR6,2001:db8::/32,PROXY,no-resolve
        URL-REGEX,"https://a,b",PROXY
        USER-AGENT,Example*,PROXY
        """, category: .proxy)
        XCTAssertEqual(result.domains, ["example.com", "+.example.org"])
        XCTAssertEqual(result.classical, ["DOMAIN-KEYWORD,google", "IP-CIDR,1.1.1.0/24,no-resolve", "IP-CIDR6,2001:db8::/32,no-resolve"])
        XCTAssertEqual(result.skipped, 2)
    }
    func testBrokenOrUnexpectedModulesCannotReplaceCache() {
        for text in ["<html>error</html>", "[Rule]\nDOMAIN,a.example,PROXY", "[Rule]\nIP-CIDR,999.1.1.1/24,DIRECT", "[Rule]\nIP-CIDR,1.1.1.1/99,DIRECT", "[Script]\nfoo", "[Rule]\nUNKNOWN,example.com,DIRECT", "[Rule]\nURL-REGEX,example,DIRECT", "#!desc=Rules:20 Created:test\n[Rule]\nDOMAIN,example.com,DIRECT"] {
            XCTAssertThrowsError(try ShadowrocketRuleParser.parse(text, category: .direct), text)
        }
    }
    func testHourlyFreshnessAndCompleteSnapshotRoundTrip() throws {
        let date = Date(timeIntervalSince1970: 10000)
        let snapshot = RuleSnapshot(modules: Dictionary(uniqueKeysWithValues: RuleCategory.allCases.map { ($0, ConvertedRuleModule(domains: ["+.\($0.rawValue).example"])) }), updatedAt: date)
        XCTAssertFalse(snapshot.isDue(at: date.addingTimeInterval(3599)))
        XCTAssertTrue(snapshot.isDue(at: date.addingTimeInterval(3600)))
        XCTAssertTrue(snapshot.isComplete)
        XCTAssertFalse(RuleSnapshot(modules: [:]).isComplete)
        XCTAssertEqual(try JSONDecoder().decode(RuleSnapshot.self, from: JSONEncoder().encode(snapshot)), snapshot)
        let data = try XCTUnwrap(snapshot.providerPayloads()["sr-direct-domain"])
        XCTAssertEqual((try Yams.load(yaml: String(decoding: data, as: UTF8.self)) as? [String: [String]])?["payload"], ["+.direct.example"])
    }
    func testOwnRepositoryPayloadsAndCacheSource() throws {
        let domains = try SmartClashRuleParser.parse("payload:\n  - '+.example.cn'\n  - 'exact.example.cn'\n  - '+.example.cn'\n", ipcidr: false)
        let ips = try SmartClashRuleParser.parse("payload:\n  - '1.0.1.0/24'\n  - '2001:db8::/32'\n", ipcidr: true)
        XCTAssertEqual(domains.domains, ["+.example.cn", "exact.example.cn"])
        XCTAssertEqual(ips.classical, ["IP-CIDR,1.0.1.0/24,no-resolve", "IP-CIDR6,2001:db8::/32,no-resolve"])
        let direct = ConvertedRuleModule(domains: domains.domains, classical: ips.classical)
        let snapshot = RuleSnapshot(modules: [.direct: direct], source: RuleSnapshot.repository)
        XCTAssertTrue(snapshot.isComplete)
        XCTAssertEqual(try JSONDecoder().decode(RuleSnapshot.self, from: JSONEncoder().encode(snapshot)), snapshot)
        XCTAssertFalse(RuleSnapshot(modules: [.direct: domains], source: RuleSnapshot.repository).isComplete)
        XCTAssertFalse(RuleSnapshot(modules: [.direct: direct, .reject: domains], source: RuleSnapshot.repository).isComplete)
        let legacy = RuleSnapshot(modules: [.direct: direct])
        XCTAssertNil(try JSONDecoder().decode(RuleSnapshot.self, from: JSONEncoder().encode(legacy)).source)
        let payload = try XCTUnwrap(snapshot.providerPayloads()["sr-direct-classical"])
        XCTAssertEqual((try Yams.load(yaml: String(decoding: payload, as: UTF8.self)) as? [String: [String]])?["payload"], ips.classical)
        for invalid in ["<html>error</html>", "payload: []", "payload: [123]", "payload: ['bad,domain']"] {
            XCTAssertThrowsError(try SmartClashRuleParser.parse(invalid, ipcidr: false))
        }
        for invalid in ["payload: ['999.1.1.1/24']", "payload: ['1.1.1.1/99']"] {
            XCTAssertThrowsError(try SmartClashRuleParser.parse(invalid, ipcidr: true))
        }
    }
    func testDownloadedUpstreamModulesConvertWhenOptedIn() throws {
        guard let directory = ProcessInfo.processInfo.environment["SMARTCLASH_RULE_FIXTURES"] else { throw XCTSkip("Opt-in downloaded upstream fixtures") }
        var modules: [RuleCategory: ConvertedRuleModule] = [:]
        for category in RuleCategory.allCases {
            modules[category] = try ShadowrocketRuleParser.parse(String(contentsOfFile: directory + "/" + category.filename, encoding: .utf8), category: category)
        }
        let snapshot = RuleSnapshot(modules: modules)
        XCTAssertTrue(snapshot.isComplete)
        print("Upstream conversion: \(snapshot.count) rules, \(snapshot.skipped) unsupported or invalid rules skipped")
        try JSONEncoder().encode(snapshot).write(to: URL(fileURLWithPath: directory + "/snapshot.json"))
        for (name, data) in try snapshot.providerPayloads() {
            try data.write(to: URL(fileURLWithPath: directory + "/" + name + ".yaml"))
        }
        let config = try Configuration.make(nodes: nodes(), secret: "fixture", tun: false, trafficMode: .rule,
            ruleProviderBaseURL: "http://127.0.0.1:27892/test/rules")
        try config.write(to: URL(fileURLWithPath: directory + "/generated-config.json"))
    }
}
