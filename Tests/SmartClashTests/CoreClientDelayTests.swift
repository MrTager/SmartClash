import XCTest
import ProxyLogic
@testable import SmartClash

private final class PendingCoreProtocol: URLProtocol {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() { }
    override func stopLoading() { }
}

private final class MockCoreProtocol: URLProtocol {
    typealias Handler = (URLRequest) throws -> (Int, Data)
    private static let lock = NSLock()
    private static var handlers: [String: Handler] = [:]
    static func register(_ handler: @escaping Handler) -> String {
        let id = UUID().uuidString
        lock.lock(); handlers[id] = handler; lock.unlock()
        return id
    }
    static func reset() { lock.lock(); handlers.removeAll(); lock.unlock() }
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let id = request.value(forHTTPHeaderField: "X-SmartClash-Test-Fixture") ?? ""
            Self.lock.lock(); let handler = Self.handlers[id]; Self.lock.unlock()
            guard let handler else { throw URLError(.cancelled) }
            let (status, data) = try handler(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

final class CoreClientDelayTests: XCTestCase {
    private func body(_ request: URLRequest) throws -> [String: String] {
        var data = request.httpBody ?? Data()
        if data.isEmpty, let stream = request.httpBodyStream {
            stream.open(); defer { stream.close() }
            var buffer = [UInt8](repeating: 0, count: 1024)
            while stream.hasBytesAvailable {
                let count = stream.read(&buffer, maxLength: buffer.count)
                if count <= 0 { break }
                data.append(contentsOf: buffer.prefix(count))
            }
        }
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: String])
    }
    @MainActor
    private func pendingLatencyStore() throws -> AppStore {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [PendingCoreProtocol.self]
        let store = AppStore(loadFromDisk: false)
        store.nodes = try SubscriptionParser.parse("proxies: [{name: fixture, type: trojan, server: example.com, port: 443, password: test}, {name: fixture2, type: trojan, server: example.org, port: 443, password: test}]", source: Source(name: "fixture", url: "https://example.com"))
        store.running = true; store.selected = store.nodes[0].name
        store.latencies[store.nodes[0].id] = NodeLatency(milliseconds: 127)
        store.api = CoreClient(secret: "fixture", session: URLSession(configuration: configuration))
        return store
    }
    @MainActor
    func testConnectionActionCanInterruptLatencyTestAndKeepOldResults() async throws {
        let store = try pendingLatencyStore()
        store.beginLatencyTest()
        try await Task.sleep(nanoseconds: 30_000_000)
        XCTAssertTrue(store.isTestingLatency)
        var executed = false
        let replacement = store.perform {
            XCTAssertFalse(store.isTestingLatency)
            executed = true
        }
        await replacement.value
        XCTAssertTrue(executed)
        XCTAssertFalse(store.busy)
        XCTAssertEqual(store.latencies[store.nodes[0].id]?.milliseconds, 127)
        XCTAssertNil(store.error)
    }
    @MainActor
    func testSingleNodeTestCanReplaceAllNodeTest() async throws {
        let store = try pendingLatencyStore()
        store.beginLatencyTest()
        try await Task.sleep(nanoseconds: 30_000_000)
        XCTAssertEqual(store.latencyTotal, 2)
        store.beginLatencyTest([store.nodes[1]])
        try await Task.sleep(nanoseconds: 30_000_000)
        XCTAssertTrue(store.isTestingLatency)
        XCTAssertEqual(store.latencyTotal, 1)
        store.cancelLatencyTest()
        await store.latencyTask?.value
        XCTAssertFalse(store.isTestingLatency)
        XCTAssertFalse(store.busy)
        XCTAssertEqual(store.latencies[store.nodes[0].id]?.milliseconds, 127)
        XCTAssertNil(store.error)
    }
    @MainActor
    func testStopCancelsLatencyTestAndRestoresOfflineState() async throws {
        let store = try pendingLatencyStore()
        store.beginLatencyTest()
        try await Task.sleep(nanoseconds: 30_000_000)
        XCTAssertTrue(store.isTestingLatency)
        let start = Date()
        store.requestStop()
        store.requestStop()
        for _ in 0..<50 {
            if !store.stopping { break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertFalse(store.stopping)
        XCTAssertFalse(store.isTestingLatency)
        XCTAssertFalse(store.running)
        XCTAssertFalse(store.busy)
        XCTAssertLessThan(Date().timeIntervalSince(start), 1)
        XCTAssertEqual(store.latencies[store.nodes[0].id]?.milliseconds, 127)
        XCTAssertNil(store.error)
    }
    private func client(_ handler: @escaping MockCoreProtocol.Handler) -> CoreClient {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockCoreProtocol.self]
        configuration.httpAdditionalHeaders = ["X-SmartClash-Test-Fixture": MockCoreProtocol.register(handler)]
        return CoreClient(secret: "test-secret", session: URLSession(configuration: configuration))
    }
    override func tearDown() { MockCoreProtocol.reset(); super.tearDown() }
    func testDelayDoesNotSwitchTheBusinessProxy() async {
        let api = client { request in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertTrue(request.url!.path.hasSuffix("/delay"))
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer test-secret")
            return (200, Data("{\"delay\":127}".utf8))
        }
        let result = await api.delay("香港 / A")
        XCTAssertEqual(result.milliseconds, 127)
        XCTAssertTrue(result.reachable)
    }
    func testCoreTimeoutIsNotDisplayedAsAZeroMillisecondSuccess() async {
        let api = client { _ in (503, Data("{\"message\":\"timeout\"}".utf8)) }
        let result = await api.delay("test")
        XCTAssertNil(result.milliseconds)
        XCTAssertFalse(result.reachable)
    }
    func testMissingDelayIsNotSuccess() async {
        let api = client { _ in (200, Data("{}".utf8)) }
        let result = await api.delay("test")
        XCTAssertFalse(result.reachable)
    }
    func testTrafficModePatchConfirmsCoreAndGlobalTracksSmart() async throws {
        var requests: [String] = []
        let api = client { request in
            requests.append("\(request.httpMethod!) \(request.url!.path)")
            if request.httpMethod == "PATCH" {
                XCTAssertEqual(try self.body(request)["mode"], "global")
            }
            return (200, Data("{\"mode\":\"global\"}".utf8))
        }
        try await api.setTrafficMode(.global)
        XCTAssertEqual(requests, ["PUT /proxies/GLOBAL", "PATCH /configs", "GET /configs"])
    }
    @MainActor
    func testFailedModeSwitchRollsBackAndDoesNotSaveRequestedMode() async throws {
        var patches: [String] = []
        let api = client { request in
            if request.httpMethod == "PATCH" {
                let mode = try XCTUnwrap(self.body(request)["mode"])
                patches.append(mode)
                if mode == "global" { return (500, Data()) }
            }
            return (200, Data("{\"mode\":\"rule\"}".utf8))
        }
        let store = AppStore(loadFromDisk: false); store.running = true; store.api = api
        store.nodes = try SubscriptionParser.parse("proxies: [{name: fixture, type: trojan, server: example.com, port: 443, password: test}]", source: Source(name: "fixture", url: "https://example.com"))
        do { try await store.changeTrafficMode(.global); XCTFail("Expected failure") } catch { }
        XCTAssertEqual(store.trafficMode, .rule)
        XCTAssertEqual(patches, ["global", "rule"])
        XCTAssertFalse(store.recovery)
    }
    @MainActor
    func testDirectSwitchClosesOldConnectionsAndSkipsNodeHealthChecks() async throws {
        var requests: [String] = []
        let api = client { request in
            requests.append("\(request.httpMethod!) \(request.url!.path)")
            return (200, Data("{\"mode\":\"direct\"}".utf8))
        }
        let store = AppStore(loadFromDisk: false); store.running = true; store.api = api
        try await store.changeTrafficMode(.direct)
        XCTAssertEqual(store.trafficMode, .direct)
        try await store.inspectCurrent(force: true)
        XCTAssertEqual(requests, ["PATCH /configs", "GET /configs", "DELETE /connections", "GET /version"])
        XCTAssertNil(store.activeMode)
    }
    @MainActor
    func testOfflineModeSwitchNeedsNoCoreAndLegacyPreferencesDefaultToRules() async throws {
        let store = AppStore(loadFromDisk: false)
        try await store.changeTrafficMode(.direct)
        XCTAssertEqual(store.preferences.trafficMode, .direct)
        let legacy = try JSONDecoder().decode(Preferences.self, from: Data("{\"sources\":[],\"interval\":60,\"automatic\":true,\"secret\":\"test\",\"subscriptionToken\":\"test\"}".utf8))
        XCTAssertEqual(legacy.trafficMode ?? .rule, .rule)
    }
    @MainActor
    func testCancelledHeartbeatDoesNotTreatCoreAsDead() async throws {
        let api = client { _ in (200, Data("{}".utf8)) }
        let store = AppStore(loadFromDisk: false)
        store.nodes = try SubscriptionParser.parse("proxies: [{name: fixture, type: trojan, server: example.com, port: 443, password: test}]", source: Source(name: "fixture", url: "https://example.com"))
        store.selected = store.nodes[0].name; store.running = true; store.api = api
        let task = Task { try await store.inspectCurrent() }
        task.cancel()
        do { try await task.value; XCTFail("Cancelled check should stop") }
        catch is CancellationError { }
        XCTAssertTrue(store.running, "Cancelling a health check must not stop a healthy core")
    }
}
