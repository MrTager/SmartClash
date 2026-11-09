import XCTest
@testable import ProxyLogic

final class RoutingAndTrafficTests: XCTestCase {
    private func pool() throws -> [ProxyNode] {
        try SubscriptionParser.parse("""
        proxies:
          - {name: Entry, type: ss, server: entry.example.com, port: 443, cipher: aes-128-gcm, password: test}
          - {name: Middle, type: trojan, server: middle.example.com, port: 443, password: test}
          - {name: Exit, type: vless, server: exit.example.com, port: 443, uuid: test}
        """, source: Source(name: "fixture", url: "https://example.com"))
    }
    func testHighSpeedRankingUsesThroughputNotLatency() throws {
        let nodes = try pool()
        let latency = [nodes[0].id: NodeLatency(milliseconds: 10), nodes[1].id: NodeLatency(milliseconds: 90)]
        let samples = [nodes[0].id: LinkMeasurements(throughput: ThroughputSample(bytesPerSecond: 100, downloadedBytes: 1_048_576)),
                       nodes[1].id: LinkMeasurements(throughput: ThroughputSample(bytesPerSecond: 900, downloadedBytes: 1_048_576))]
        XCTAssertEqual(ModeRanking.best(nodes, mode: .highSpeed, latencies: latency, measurements: samples).map(\.id), [nodes[1].id, nodes[0].id])
        XCTAssertEqual(ModeRanking.best(nodes, mode: .lowLatency, latencies: latency, measurements: samples).first?.id, nodes[0].id)
    }
    func testAIModeRequiresPositiveVerification() throws {
        let nodes = try pool()
        let samples = [nodes[0].id: LinkMeasurements(openAIReachable: false), nodes[2].id: LinkMeasurements(openAIReachable: true)]
        XCTAssertEqual(ModeRanking.best(nodes, mode: .ai, latencies: [:], measurements: samples).map(\.id), [nodes[2].id])
    }
    func testHighSpeedModeDoesNotInventUnmeasuredBandwidth() throws {
        XCTAssertTrue(ModeRanking.best(try pool(), mode: .highSpeed, latencies: [:], measurements: [:]).isEmpty)
        XCTAssertEqual(ThroughputSample(bytesPerSecond: 1_000_000, downloadedBytes: 1_048_576).megabitsPerSecond, 8)
    }
    func testAllModesPreferNonHongKongWithoutSkippingVerification() {
        let source = UUID()
        let hk = ProxyNode(id: "hk", name: "HK01", sourceID: source, payload: [:])
        let jp = ProxyNode(id: "jp", name: "JP01", sourceID: source, payload: [:])
        let nodes = [hk, jp]
        let latencies = ["hk": NodeLatency(milliseconds: 5), "jp": NodeLatency(milliseconds: 90)]
        var samples = ["hk": LinkMeasurements(throughput: ThroughputSample(bytesPerSecond: 900, downloadedBytes: 1_048_576), openAIReachable: true),
                       "jp": LinkMeasurements(throughput: ThroughputSample(bytesPerSecond: 100, downloadedBytes: 1_048_576), openAIReachable: true)]
        for mode in RoutingMode.allCases {
            XCTAssertEqual(ModeRanking.best(nodes, mode: mode, latencies: latencies, measurements: samples).map(\.id), ["jp", "hk"])
        }
        samples["jp"]?.openAIReachable = false
        XCTAssertEqual(ModeRanking.best(nodes, mode: .ai, latencies: latencies, measurements: samples).map(\.id), ["hk"])
        XCTAssertTrue(ModeRanking.best(nodes, mode: .ai, latencies: latencies, measurements: [:]).isEmpty)
    }
    func testActualDialerReferencesBecomeOrderedHops() throws {
        let raw = try pool()
        var middle = raw[1].payload; middle["name"] = "AUTO-middle"; middle["dialer-proxy"] = raw[0].name
        var exit = raw[2].payload; exit["name"] = "AUTO-exit"; exit["dialer-proxy"] = "AUTO-middle"
        let definitions = [ProxyNode(id: "auto-middle", name: "AUTO-middle", sourceID: raw[1].sourceID, payload: middle),
                           ProxyNode(id: "auto-exit", name: "AUTO-exit", sourceID: raw[2].sourceID, payload: exit)]
        let trace = ChainTrace.resolve(selected: "AUTO-exit", rawNodes: raw, definitions: definitions)
        XCTAssertTrue(trace.complete)
        XCTAssertEqual(trace.hops.map(\.name), raw.map(\.name))
    }
    func testBrokenReferenceIsNotDisplayedAsCompletePath() throws {
        let raw = try pool(); var payload = raw[0].payload; payload["dialer-proxy"] = "missing"
        let node = ProxyNode(id: raw[0].id, name: raw[0].name, sourceID: raw[0].sourceID, payload: payload)
        XCTAssertFalse(ChainTrace.resolve(selected: node.name, rawNodes: [node], definitions: []).complete)
        payload["dialer-proxy"] = node.name
        let cycle = ProxyNode(id: node.id, name: node.name, sourceID: node.sourceID, payload: payload)
        XCTAssertFalse(ChainTrace.resolve(selected: node.name, rawNodes: [cycle], definitions: []).complete)
    }
    func testDisconnectedTraceHasNoFakeHops() throws {
        let trace = ChainTrace.resolve(selected: "", rawNodes: try pool(), definitions: [])
        XCTAssertTrue(trace.hops.isEmpty); XCTAssertFalse(trace.complete)
    }
    func testTrafficTotalsAccumulateDeltasNotEverySnapshot() {
        var ledger = TrafficLedger()
        ledger.record(upload: 100, download: 500, runID: "a")
        ledger.record(upload: 120, download: 600, runID: "a")
        ledger.record(upload: 120, download: 600, runID: "a")
        XCTAssertEqual(ledger.lifetimeUpload, 120)
        XCTAssertEqual(ledger.lifetimeDownload, 600)
    }
    func testTrafficResetAndNewCoreNeverSubtractTotals() {
        var ledger = TrafficLedger()
        ledger.record(upload: 100, download: 500, runID: "a")
        ledger.record(upload: 10, download: 20, runID: "a")
        ledger.record(upload: 5, download: 8, runID: "b")
        XCTAssertEqual(ledger.lifetimeUpload, 115)
        XCTAssertEqual(ledger.lifetimeDownload, 528)
    }
    func testTrafficPersistenceDoesNotDoubleCountSameSample() throws {
        var ledger = TrafficLedger(); ledger.record(upload: 100, download: 500, runID: "a")
        var restored = try JSONDecoder().decode(TrafficLedger.self, from: JSONEncoder().encode(ledger))
        restored.record(upload: 100, download: 500, runID: "a")
        XCTAssertEqual(restored.lifetimeUpload, 100)
        XCTAssertEqual(restored.lifetimeDownload, 500)
    }
    func testTrafficDecoderMatchesCoreFields() throws {
        let sample = try JSONDecoder().decode(TrafficSample.self, from: Data("{\"up\":12,\"down\":64,\"upTotal\":100,\"downTotal\":500}".utf8))
        XCTAssertEqual(sample.up, 12); XCTAssertEqual(sample.downTotal, 500)
    }
}
