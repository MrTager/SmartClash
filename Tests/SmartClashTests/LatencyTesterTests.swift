import XCTest
import ProxyLogic
@testable import SmartClash

private actor ProbeCounter {
    var active = 0
    var peak = 0
    var started = 0
    func begin() { active += 1; started += 1; peak = max(peak, active) }
    func end() { active -= 1 }
    func snapshot() -> (Int, Int, Int) { (active, peak, started) }
}

final class LatencyTesterTests: XCTestCase {
    @MainActor
    func testBoundedConcurrencyAndEveryResultArrives() async {
        let counter = ProbeCounter()
        let nodes = (0..<20).map { LatencyRequest(id: String($0), name: "test") }
        var results: [String: NodeLatency] = [:]
        await LatencyTester.run(nodes, concurrency: 3, probe: { _ in
            await counter.begin()
            try? await Task.sleep(nanoseconds: 2_000_000)
            await counter.end()
            return NodeLatency(milliseconds: 15)
        }, receive: { id, value in results[id] = value })
        let counts = await counter.snapshot()
        XCTAssertEqual(results.count, 20)
        XCTAssertEqual(counts.0, 0)
        XCTAssertLessThanOrEqual(counts.1, 3)
    }
    @MainActor
    func testCancellationStopsSchedulingNewProbes() async {
        let counter = ProbeCounter()
        let nodes = (0..<100).map { LatencyRequest(id: String($0), name: "test") }
        let task = Task {
            await LatencyTester.run(nodes, concurrency: 3, probe: { _ in
                await counter.begin()
                try? await Task.sleep(nanoseconds: 200_000_000)
                await counter.end()
                return NodeLatency(milliseconds: 15)
            }, receive: { _, _ in })
        }
        try? await Task.sleep(nanoseconds: 10_000_000)
        task.cancel(); await task.value
        let counts = await counter.snapshot()
        XCTAssertEqual(counts.0, 0)
        XCTAssertLessThanOrEqual(counts.2, 3)
    }
    func testDefaultInterfaceParserDoesNotGuessWiFi() {
        XCTAssertEqual(NetworkEnvironment.parseDefaultInterface("route to: default\n  interface: en11\n"), "en11")
        XCTAssertNil(NetworkEnvironment.parseDefaultInterface("interface: en0;bad"))
        XCTAssertNil(NetworkEnvironment.parseDefaultInterface("no default route"))
    }
    func testOldPreferencesRemainDecodable() throws {
        let data = Data("{\"sources\":[],\"interval\":60,\"automatic\":true,\"secret\":\"test\",\"subscriptionToken\":\"test\"}".utf8)
        let preferences = try JSONDecoder().decode(Preferences.self, from: data)
        XCTAssertNotEqual(preferences.exploreMultihop, true)
    }
    func testLatencyOnlyCoreDoesNotStartConnectionMonitor() {
        XCTAssertFalse(CoreStartPurpose.latencyTest.monitorsConnections)
        XCTAssertTrue(CoreStartPurpose.connection.monitorsConnections)
    }
}
