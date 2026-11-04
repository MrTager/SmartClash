import XCTest
@testable import ProxyLogic

final class ConnectionSnapshotTests: XCTestCase {
    func testProxyPeerIsNeverMistakenForWebsiteDestination() throws {
        let snapshot = try JSONDecoder().decode(ConnectionSnapshot.self, from: Data("""
        {"uploadTotal":1024,"downloadTotal":2048,"connections":[{"id":"request","metadata":{"network":"tcp","host":"example.com","destinationIP":"","remoteDestination":"1.1.1.1","destinationPort":"443"},"upload":12,"download":64,"chains":["Tokyo","SMART"],"rule":"Match"}]}
        """.utf8))
        XCTAssertNil(snapshot.connections.first?.metadata.publicDestinationIP)
        XCTAssertEqual(snapshot.connections.first?.metadata.geolocationKey, "example.com")
        XCTAssertEqual(snapshot.connections.first?.metadata.displayDestination, "example.com")
        XCTAssertEqual(snapshot.uploadTotal, 1024)
    }
    func testNullConnectionsAreAnEmptySnapshot() throws {
        let value = try JSONDecoder().decode(ConnectionSnapshot.self, from: Data("{\"uploadTotal\":0,\"downloadTotal\":0,\"connections\":null}".utf8))
        XCTAssertTrue(value.connections.isEmpty)
    }
    func testPrivateAndFakeAddressesNeverReachGeoIPProvider() {
        for ip in ["", "localhost", "127.0.0.1", "10.1.1.1", "192.168.1.1", "172.16.0.1", "169.254.1.1", "100.64.1.1", "198.18.0.1", "203.0.113.1", "224.0.0.1", "::1", "fe80::1", "fc00::1", "2001:db8::1", "::ffff:192.168.1.1"] {
            XCTAssertFalse(PublicIPAddress.isPublic(ip), ip)
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            XCTAssertTrue(PublicIPAddress.isPublic(ip), ip)
        }
        XCTAssertNil(ConnectionMetadata(destinationIP: "198.18.0.1", remoteDestination: "8.8.8.8").publicDestinationIP)
    }
    private func node(_ name: String, dialer: String? = nil) -> ProxyNode {
        var payload: [String: Any] = ["name": name, "server": name + ".example.com"]
        payload["dialer-proxy"] = dialer
        return ProxyNode(id: name, name: name, sourceID: UUID(), payload: payload)
    }
    func testConnectionRouteRetainsItsOwnExitAfterSelectedGroupChanges() {
        let old = node("old"), new = node("new")
        let connection = CoreConnection(id: "old-request", metadata: ConnectionMetadata(), chains: ["old", "SMART"])
        XCTAssertEqual(connection.routeNodes(in: [new, old]).map(\.name), ["old"])
    }
    func testRealDialerReferencesResolveInEntryToExitOrder() {
        let entry = node("entry"), exit = node("exit", dialer: "entry")
        let connection = CoreConnection(id: "two-hop", metadata: ConnectionMetadata(), chains: ["exit", "SMART"])
        XCTAssertEqual(connection.routeNodes(in: [entry, exit]).map(\.name), ["entry", "exit"])
        XCTAssertTrue(connection.routeNodes(in: [exit]).isEmpty, "Missing hops must not become a fabricated complete route")
        let cycle = node("exit", dialer: "exit")
        XCTAssertTrue(connection.routeNodes(in: [cycle]).isEmpty)
    }
    func testDirectAndRejectedConnectionsHaveNoProxyRoute() {
        let exit = node("exit")
        for chain in [["DIRECT"], ["REJECT"], ["unknown", "SMART"]] {
            XCTAssertTrue(CoreConnection(id: "test", metadata: ConnectionMetadata(), chains: chain).routeNodes(in: [exit]).isEmpty)
        }
    }
}
