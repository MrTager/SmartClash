import XCTest
import ProxyLogic
@testable import SmartClash

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
