import XCTest
import ProxyLogic
@testable import SmartClash

final class ActivityAndStreamingTests: XCTestCase {
    func testNetworkCommandCancellationIsPrompt() async throws {
        let start = Date()
        let task = Task { try await Command.run("/bin/sleep", ["5"], cancellable: true) }
        try await Task.sleep(nanoseconds: 30_000_000)
        task.cancel()
        do { _ = try await task.value; XCTFail("Cancellation must throw") }
        catch is CancellationError { }
        XCTAssertLessThan(Date().timeIntervalSince(start), 2)
    }
    @MainActor
    func testBackgroundMonitoringDoesNotOwnOperationState() async throws {
        let store = AppStore(loadFromDisk: false)
        XCTAssertFalse(store.busy)
        store.publishRoute(); store.logBackground("background fixture")
        XCTAssertFalse(store.busy)
        store.perform { try await Task.sleep(nanoseconds: 30_000_000) }
        XCTAssertTrue(store.busy)
        try await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertFalse(store.busy)
    }
    @MainActor
    func testNewActionCancelsPreviousAndWaitsForItsCleanup() async throws {
        let store = AppStore(loadFromDisk: false)
        var events: [String] = []
        store.perform {
            events.append("started")
            do { try await Task.sleep(nanoseconds: 5_000_000_000) }
            catch {
                await Task.detached { try? await Task.sleep(nanoseconds: 30_000_000) }.value
                events.append("cleaned up")
                throw error
            }
        }
        try await Task.sleep(nanoseconds: 20_000_000)
        let next = store.perform {
            XCTAssertTrue(store.busy, "Old cleanup must not clear the new operation's state")
            events.append("replacement")
        }
        await next.value
        XCTAssertEqual(events, ["started", "cleaned up", "replacement"])
        XCTAssertNil(store.error)
        XCTAssertFalse(store.busy)
    }
    @MainActor
    func testActionRequestedWhileStoppingWaitsForNetworkRestoration() async throws {
        let store = AppStore(loadFromDisk: false)
        store.running = true
        store.perform { try await Task.sleep(nanoseconds: 5_000_000_000) }
        try await Task.sleep(nanoseconds: 20_000_000)
        store.requestStop()
        var executed = false
        let next = store.perform {
            XCTAssertFalse(store.stopping)
            XCTAssertFalse(store.running)
            executed = true
        }
        await next.value
        XCTAssertTrue(executed)
        XCTAssertFalse(store.busy)
        XCTAssertNil(store.error)
    }
    @MainActor
    func testRepeatedStopAlsoCancelsQueuedActions() async throws {
        let store = AppStore(loadFromDisk: false)
        store.perform { try await Task.sleep(nanoseconds: 5_000_000_000) }
        try await Task.sleep(nanoseconds: 20_000_000)
        store.requestStop()
        var executed = false
        let queued = store.perform { executed = true }
        store.requestStop()
        await queued.value
        XCTAssertFalse(executed)
        XCTAssertFalse(store.stopping)
        XCTAssertFalse(store.busy)
    }
    @MainActor
    func testStopPreemptsForegroundWorkWithoutTouchingRealNetwork() async throws {
        let store = AppStore(loadFromDisk: false)
        store.perform { try await Task.sleep(nanoseconds: 5_000_000_000) }
        try await Task.sleep(nanoseconds: 20_000_000)
        let start = Date(); store.requestStop()
        for _ in 0..<20 {
            if !store.stopping { break }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertFalse(store.stopping)
        XCTAssertFalse(store.busy)
        XCTAssertLessThan(Date().timeIntervalSince(start), 1)
    }
    @MainActor
    func testLegacyPreferencesDefaultToLowLatency() throws {
        let preferences = try JSONDecoder().decode(Preferences.self, from: Data("{\"sources\":[],\"interval\":60,\"automatic\":true,\"secret\":\"test\",\"subscriptionToken\":\"test\"}".utf8))
        XCTAssertEqual(preferences.routingMode ?? .lowLatency, .lowLatency)
    }
    func testLiveTrafficStreamReadOnly() async throws {
        guard ProcessInfo.processInfo.environment["SMARTCLASH_LIVE_TEST"] == "1" else {
            throw XCTSkip("Opt-in read-only live-core integration test")
        }
        let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("SmartClash")
        let document = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("preferences.json"))) as! [String: Any]
        let client = CoreClient(secret: document["secret"] as! String)
        _ = try await client.request("/version")
        let sample = try await withThrowingTaskGroup(of: TrafficSample.self) { group in
            group.addTask {
                for try await sample in client.trafficStream() { return sample }
                throw ProxyError.message("Stream ended before first sample")
            }
            group.addTask {
                try await Task.sleep(nanoseconds: 5_000_000_000)
                throw ProxyError.message("Traffic stream timeout")
            }
            let first = try await group.next()!
            group.cancelAll(); return first
        }
        XCTAssertNotNil(sample.upTotal)
        XCTAssertNotNil(sample.downTotal)
    }
    @MainActor
    func testLiveTrafficStoreFeedsPanelWithoutPersistentWrites() async throws {
        guard ProcessInfo.processInfo.environment["SMARTCLASH_LIVE_TEST"] == "1" else { throw XCTSkip("Opt-in live-core integration") }
        let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("SmartClash")
        let preferences = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("preferences.json"))) as! [String: Any]
        let statistics = TrafficStore(filename: nil)
        defer { statistics.stop() }
        statistics.start(client: CoreClient(secret: preferences["secret"] as! String), runID: "read-only-test")
        for _ in 0..<50 {
            if statistics.connected { break }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        XCTAssertTrue(statistics.connected)
        XCTAssertFalse(statistics.points.isEmpty)
        XCTAssertEqual(statistics.lifetimeUpload, statistics.sessionUpload)
        XCTAssertEqual(statistics.lifetimeDownload, statistics.sessionDownload)
    }
}
