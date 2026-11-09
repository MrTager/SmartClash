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
        let operation = store.perform { try await Task.sleep(nanoseconds: 30_000_000) }
        XCTAssertTrue(store.busy)
        // Wait for completion rather than racing the CI runner's scheduler.
        await operation.value
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
    func testGeolocationCandidatesPrioritizeExitBeforeEntryAndOldConnections() {
        let entry = ProxyNode(id: "entry", name: "Entry", sourceID: UUID(), payload: ["server": "entry.example.test"])
        let exit = ProxyNode(id: "exit", name: "Exit", sourceID: UUID(), payload: ["server": "exit.example.test"])
        let old = ProxyNode(id: "old", name: "Old", sourceID: UUID(), payload: ["server": "old.example.test"])
        let standby = ProxyNode(id: "standby", name: "Standby", sourceID: UUID(), payload: ["server": "standby.example.test"])
        let activity = NetworkActivityStore(geo: GeoIPService(cacheURL: nil))
        activity.configure(nodes: [standby, old, entry, exit], active: [entry, exit])
        activity.ingest(ConnectionSnapshot(uploadTotal: 0, downloadTotal: 0, connections: [
            CoreConnection(id: "old-request", metadata: ConnectionMetadata(destinationIP: "1.1.1.1"), chains: [old.name])
        ]))
        let date = Date(timeIntervalSince1970: 1_000)
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "exit.example.test")
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "entry.example.test")
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "old.example.test")
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "1.1.1.1")
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "standby.example.test")
        XCTAssertNil(activity.nextGeolocationHost(at: date))
        XCTAssertFalse(activity.enabled)
        XCTAssertFalse(activity.resolving)
    }
    @MainActor
    func testPreferredGeolocationFailureRetriesAfterTenSeconds() {
        let node = ProxyNode(id: "exit", name: "Exit", sourceID: UUID(), payload: ["server": "exit.example.test"])
        let activity = NetworkActivityStore(geo: GeoIPService(cacheURL: nil))
        activity.configure(nodes: [node], active: [node])
        let date = Date(timeIntervalSince1970: 1_000)
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "exit.example.test")
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(9)))
        XCTAssertEqual(activity.nextGeolocationHost(at: date.addingTimeInterval(10)), "exit.example.test")
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(19)))
        XCTAssertEqual(activity.nextGeolocationHost(at: date.addingTimeInterval(20)), "exit.example.test")
    }
    @MainActor
    func testNonpreferredGeolocationFailureKeepsFiveMinuteBackoff() {
        let node = ProxyNode(id: "standby", name: "Standby", sourceID: UUID(), payload: ["server": "standby.example.test"])
        let activity = NetworkActivityStore(geo: GeoIPService(cacheURL: nil))
        activity.configure(nodes: [node], active: [])
        let date = Date(timeIntervalSince1970: 1_000)
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "standby.example.test")
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(299)))
        XCTAssertEqual(activity.nextGeolocationHost(at: date.addingTimeInterval(300)), "standby.example.test")
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(599)))
        XCTAssertEqual(activity.nextGeolocationHost(at: date.addingTimeInterval(600)), "standby.example.test")
    }
    @MainActor
    func testRouteChangeClearsOnlyCurrentServersBackoffOnce() {
        let entry = ProxyNode(id: "entry", name: "Entry", sourceID: UUID(), payload: ["server": "entry.example.test"])
        let a = ProxyNode(id: "A", name: "A", sourceID: UUID(), payload: ["server": "a.example.test"])
        let b = ProxyNode(id: "B", name: "B", sourceID: UUID(), payload: ["server": "b.example.test"])
        let standby = ProxyNode(id: "standby", name: "Standby", sourceID: UUID(), payload: ["server": "standby.example.test"])
        let nodes = [entry, a, b, standby]
        let activity = NetworkActivityStore(geo: GeoIPService(cacheURL: nil))
        activity.configure(nodes: nodes, active: [entry, a])
        let date = Date(timeIntervalSince1970: 1_000)
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "a.example.test")
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "entry.example.test")
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "b.example.test")
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "standby.example.test")
        activity.configure(nodes: nodes, active: [entry, a])
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(1)))

        activity.configure(nodes: nodes, active: [entry, b])
        let switched = date.addingTimeInterval(2)
        XCTAssertEqual(activity.nextGeolocationHost(at: switched), "b.example.test")
        XCTAssertEqual(activity.nextGeolocationHost(at: switched), "entry.example.test")
        XCTAssertNil(activity.nextGeolocationHost(at: switched))
        for _ in 0..<3 {
            activity.configure(nodes: Array(nodes.reversed()), active: [entry, b])
            XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(3)))
        }
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(11)))
        XCTAssertEqual(activity.nextGeolocationHost(at: date.addingTimeInterval(12)), "b.example.test")
    }
    @MainActor
    func testSharedServerNodeIdentityChangeClearsBackoffButRepeatedConfigureDoesNot() {
        let a = ProxyNode(id: "A", name: "A", sourceID: UUID(), payload: ["server": "shared.example.test"])
        let b = ProxyNode(id: "B", name: "B", sourceID: UUID(), payload: ["server": "shared.example.test"])
        let nodes = [a, b]
        let activity = NetworkActivityStore(geo: GeoIPService(cacheURL: nil))
        let date = Date(timeIntervalSince1970: 1_000)
        activity.configure(nodes: nodes, active: [a])
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "shared.example.test")
        activity.configure(nodes: nodes, active: [a])
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(1)))
        activity.configure(nodes: nodes, active: [b])
        XCTAssertEqual(activity.nextGeolocationHost(at: date.addingTimeInterval(2)), "shared.example.test")
        activity.configure(nodes: nodes, active: [b])
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(3)))
    }
    @MainActor
    func testServerChangeForSameNodeAndLeavingPreferredRouteUpdateBackoff() {
        let original = ProxyNode(id: "exit", name: "Exit", sourceID: UUID(), payload: ["server": "old.example.test"])
        let updated = ProxyNode(id: original.id, name: original.name, sourceID: original.sourceID, payload: ["server": "new.example.test"])
        let activity = NetworkActivityStore(geo: GeoIPService(cacheURL: nil))
        let date = Date(timeIntervalSince1970: 1_000)
        activity.configure(nodes: [original, updated], active: [original])
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "old.example.test")
        XCTAssertEqual(activity.nextGeolocationHost(at: date), "new.example.test")
        activity.configure(nodes: [updated], active: [updated])
        XCTAssertEqual(activity.nextGeolocationHost(at: date.addingTimeInterval(1)), "new.example.test")
        activity.configure(nodes: [updated], active: [])
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(11)))
        XCTAssertNil(activity.nextGeolocationHost(at: date.addingTimeInterval(300)))
        XCTAssertEqual(activity.nextGeolocationHost(at: date.addingTimeInterval(301)), "new.example.test")
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
