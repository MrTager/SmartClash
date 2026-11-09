import XCTest
@testable import SmartClashManager

final class MobileLibraryTests: XCTestCase {
    private let fixture = """
    proxies:
      - {name: first, type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: fixture-not-a-real-password}
      - {name: duplicate, type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: fixture-not-a-real-password}
      - {name: 剩余流量, type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: informational}
    """
    private func sample() -> MobileLibrary {
        var library = MobileLibrary()
        library.subscriptions = [.init(source: Source(name: "离线测试", url: ""), content: fixture, updatedAt: Date())]
        return library
    }
    func testUsesDesktopParserDedupAndInformationFilter() throws {
        let nodes = try MobileConfiguration.parsed(sample())
        XCTAssertEqual(nodes.count, 1)
        XCTAssertFalse(nodes[0].name.contains("剩余"))
    }
    func testExportRequiresSelectedProxyNodeButDirectWorksWithoutNode() throws {
        var library = MobileLibrary()
        XCTAssertThrowsError(try MobileConfiguration.export(library: library, node: nil))
        library.trafficMode = .direct
        let data = try MobileConfiguration.export(library: library, node: nil)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(object["mode"] as? String, "direct")
        XCTAssertNil(object["external-controller"])
        XCTAssertNil(object["script"])
        XCTAssertFalse(try XCTUnwrap(object["allow-lan"] as? Bool))
    }
    func testExportContainsOnlyChosenNode() throws {
        let library = sample()
        let node = try XCTUnwrap(MobileConfiguration.parsed(library).first)
        let data = try MobileConfiguration.export(library: library, node: node)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let proxies = try XCTUnwrap(object["proxies"] as? [[String: Any]])
        XCTAssertEqual(proxies.count, 1)
        XCTAssertEqual(proxies[0]["name"] as? String, node.name)
    }
    func testLibraryRoundTripAndRejectUnsupportedVersionDuplicateIDs() throws {
        let library = sample()
        let restored = try JSONDecoder().decode(MobileLibrary.self, from: JSONEncoder().encode(library))
        try MobileConfiguration.validate(restored)
        var invalid = restored
        invalid.version = 99
        XCTAssertThrowsError(try MobileConfiguration.validate(invalid))
        invalid = restored
        invalid.subscriptions.append(restored.subscriptions[0])
        XCTAssertThrowsError(try MobileConfiguration.validate(invalid))
    }
    func testRejectUnsafeDownloadURLs() throws {
        for address in ["http://example.com/sub", "file:///tmp/sub", "https://user:pass@example.com/sub", "https://example.com/sub#token"] {
            XCTAssertThrowsError(try SubscriptionDownload.validateURL(address), address)
        }
        XCTAssertEqual(try SubscriptionDownload.validateURL("https://example.com/sub?token=fixture").scheme, "https")
    }
    func testParserRejectsScriptAndPluginFields() throws {
        let source = Source(name: "test", url: "")
        XCTAssertThrowsError(try SubscriptionParser.parse("proxies: [{name: unsafe, type: ss, server: example.com, port: 443, script: bad}]", source: source))
    }
    @MainActor
    func testVPNCannotPretendToBeConnectedWithoutEngine() throws {
        let tunnel = TunnelController()
        XCTAssertFalse(tunnel.canStart)
        XCTAssertFalse(tunnel.connected)
        XCTAssertThrowsError(try tunnel.start())
    }
}
