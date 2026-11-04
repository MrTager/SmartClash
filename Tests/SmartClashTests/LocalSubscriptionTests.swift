import XCTest
import ProxyLogic
@testable import SmartClash

final class LocalSubscriptionTests: XCTestCase {
    private let fixture = """
    proxies:
      - {name: Test, type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: test}
    """

    func testLocalSnapshotRefreshDoesNotDownload() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = Source(name: "local", url: "")
        try Data(fixture.utf8).write(to: directory.appendingPathComponent("subscription-\(source.id).yaml"))
        let update = try await SubscriptionService.update([source], throughProxy: false, cacheDirectory: directory)
        XCTAssertEqual(update.nodes.count, 1)
        XCTAssertTrue(update.warnings.isEmpty)
        XCTAssertEqual(SubscriptionService.cached([source], cacheDirectory: directory).count, 1)
    }

    func testMissingLocalSnapshotFailsWithoutNetwork() async {
        let source = Source(name: "missing", url: "")
        do {
            _ = try await SubscriptionService.update([source], throughProxy: false,
                cacheDirectory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
            XCTFail("Missing local snapshot must not succeed")
        } catch { XCTAssertTrue(error.localizedDescription.contains("missing")) }
    }

    func testLocalSourceCodableCompatibility() throws {
        let local = Source(name: "offline", url: "")
        let roundTrip = try JSONDecoder().decode(Source.self, from: JSONEncoder().encode(local))
        XCTAssertTrue(roundTrip.isLocal)
        XCTAssertEqual(roundTrip, local)
        XCTAssertFalse(Source(name: "remote", url: "https://example.com").isLocal)
    }
}
