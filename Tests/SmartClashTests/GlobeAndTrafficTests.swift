import XCTest
import simd
import AppKit
import SceneKit
@testable import ProxyLogic
@testable import SmartClash

final class GlobeAndTrafficTests: XCTestCase {
    @MainActor
    func testTrafficFallbackComputesActualCounterDeltaAndDoesNotDoubleCount() {
        let traffic = TrafficStore(filename: nil), start = Date()
        traffic.ingestSnapshot(ConnectionSnapshot(uploadTotal: 100, downloadTotal: 200, connections: []), runID: "test", at: start)
        traffic.ingestSnapshot(ConnectionSnapshot(uploadTotal: 200, downloadTotal: 600, connections: []), runID: "test", at: start.addingTimeInterval(2))
        XCTAssertEqual(traffic.uploadRate, 50); XCTAssertEqual(traffic.downloadRate, 200)
        XCTAssertEqual(traffic.lifetimeUpload, 200); XCTAssertEqual(traffic.lifetimeDownload, 600)
        XCTAssertTrue(traffic.connected); XCTAssertTrue(traffic.usingFallback)
        traffic.ingest(TrafficSample(up: 20, down: 40, upTotal: 200, downTotal: 600), runID: "test", at: start.addingTimeInterval(3))
        traffic.ingestSnapshot(ConnectionSnapshot(uploadTotal: 200, downloadTotal: 600, connections: []), runID: "test", at: start.addingTimeInterval(4))
        XCTAssertEqual(traffic.uploadRate, 20); XCTAssertFalse(traffic.usingFallback)
        XCTAssertEqual(traffic.lifetimeDownload, 600)
    }
    @MainActor
    func testOlderCrossedSnapshotDoesNotResetTotalsOrInflateLifetime() {
        let traffic = TrafficStore(filename: nil)
        traffic.ingest(TrafficSample(up: 0, down: 0, upTotal: 100, downTotal: 200), runID: "test")
        traffic.ingestSnapshot(ConnectionSnapshot(uploadTotal: 100, downloadTotal: 150, connections: []), runID: "test")
        traffic.ingestSnapshot(ConnectionSnapshot(uploadTotal: 110, downloadTotal: 210, connections: []), runID: "test")
        XCTAssertEqual(traffic.lifetimeUpload, 110); XCTAssertEqual(traffic.lifetimeDownload, 210)
        XCTAssertEqual(traffic.sessionDownload, 210)
    }
    @MainActor
    func testDisconnectZerosSpeedsAndKeepsHistoricalCounters() {
        let traffic = TrafficStore(filename: nil), start = Date()
        traffic.ingest(TrafficSample(up: 20, down: 40, upTotal: 100, downTotal: 200), runID: "test", at: start)
        traffic.markUnavailable(at: start.addingTimeInterval(5))
        XCTAssertFalse(traffic.connected); XCTAssertEqual(traffic.downloadRate, 0)
        traffic.stop()
        XCTAssertEqual(traffic.lifetimeDownload, 200)
    }
    func testMapOnlyDrawsKnownProxyToKnownActualDestination() {
        let node = ProxyNode(id: "Tokyo", name: "Tokyo", sourceID: UUID(), payload: ["server": "node.example.com"])
        let connection = CoreConnection(id: "request", metadata: ConnectionMetadata(destinationIP: "1.1.1.1"), chains: [node.name, "SMART"])
        let locations = ["node.example.com": GeoLocation(latitude: 35, longitude: 139, country: "Japan"),
                         "1.1.1.1": GeoLocation(latitude: -27, longitude: 153, country: "Australia")]
        let trace = ChainTrace.resolve(selected: node.name, rawNodes: [node], definitions: [])
        let data = GlobeSnapshot.make(nodes: [node], activeTrace: trace, connections: [connection], locations: locations, focusedConnection: nil)
        XCTAssertEqual(data.mappedConnections, 1); XCTAssertEqual(data.routes.count, 1)
        XCTAssertEqual(data.routes.first?.locations, [locations["node.example.com"]!, locations["1.1.1.1"]!])
        let unknown = GlobeSnapshot.make(nodes: [node], activeTrace: trace, connections: [connection], locations: [:], focusedConnection: nil)
        XCTAssertTrue(unknown.routes.isEmpty)
        let stopped = GlobeSnapshot.make(nodes: [node], activeTrace: trace, connections: [], locations: locations, focusedConnection: nil)
        XCTAssertTrue(stopped.routes.isEmpty)
    }
    @MainActor
    func testGlobeArcHandlesSameLocationAntipodesAndDateLineWithoutNaN() {
        for (a, b) in [(0.0, 0.0), (0, 180), (179, -179)] {
            let points = GlobeScene.arc(from: GeoLocation(latitude: 0, longitude: a, country: ""), to: GeoLocation(latitude: 0, longitude: b, country: ""))
            XCTAssertTrue(points.allSatisfy { $0.x.isFinite && $0.y.isFinite && $0.z.isFinite })
            XCTAssertTrue(points.allSatisfy { simd_length($0) >= 1 })
        }
    }
    @MainActor
    func testGlobeStaysCenteredAndInsideViewportAfterRepeatedScrollAndRotation() {
        _ = NSApplication.shared
        let globe = GlobeScene()
        let view = GlobeSCNView(renderer: globe)
        for size in [NSSize(width: 900, height: 330), NSSize(width: 640, height: 270)] {
            view.frame = NSRect(origin: .zero, size: size)
            for amount: Float in [0.5, -0.5, 1000, -1000] {
                for _ in 0..<100 {
                    globe.zoomCamera(by: amount)
                    globe.rotateCamera(horizontal: 13, vertical: CGFloat(amount > 0 ? 30 : -30))
                }
                _ = view.snapshot()
                let center = view.projectPoint(SCNVector3Zero)
                XCTAssertEqual(center.x, size.width / 2, accuracy: 0.01)
                XCTAssertEqual(center.y, size.height / 2, accuracy: 0.01)
                for latitude in stride(from: -90.0, through: 90, by: 15) {
                    for longitude in stride(from: -180.0, to: 180, by: 15) {
                        let point = view.projectPoint(SCNVector3(GlobeScene.position(
                            GeoLocation(latitude: latitude, longitude: longitude, country: ""))))
                        XCTAssertTrue(point.x >= 0 && point.x <= size.width)
                        XCTAssertTrue(point.y >= 0 && point.y <= size.height)
                        XCTAssertTrue(point.z > 0 && point.z < 1)
                    }
                }
            }
        }
    }
    @MainActor
    func testResetRestoresRenderedCameraAfterZoomAndRotation() {
        _ = NSApplication.shared
        let globe = GlobeScene()
        let view = GlobeSCNView(renderer: globe)
        let location = GeoLocation(latitude: 35.68, longitude: 139.76, country: "Japan")
        globe.resetCamera(toward: location)
        let originalOrientation = globe.camera.simdOrientation
        globe.zoomCamera(by: -1000)
        globe.rotateCamera(horizontal: 240, vertical: 140)
        globe.resetCamera(toward: location)
        XCTAssertTrue(view.pointOfView === globe.camera)
        XCTAssertFalse(view.allowsCameraControl)
        XCTAssertLessThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(location) * 3.9), 0.0001)
        XCTAssertGreaterThan(abs(simd_dot(originalOrientation.vector, globe.camera.simdOrientation.vector)), 0.9999)
        XCTAssertGreaterThan(simd_dot(globe.camera.simdWorldFront, -simd_normalize(globe.camera.simdPosition)), 0.9999)
    }
    @MainActor
    func testPointerDragRotatesAroundGlobeWithoutChangingDistance() throws {
        _ = NSApplication.shared
        let globe = GlobeScene()
        let view = GlobeSCNView(renderer: globe)
        let start = globe.camera.simdPosition
        let down = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: NSPoint(x: 100, y: 100),
            modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
        let drag = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDragged, location: NSPoint(x: 240, y: 200),
            modifierFlags: [], timestamp: 1, windowNumber: 0, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        view.mouseDown(with: down)
        view.mouseDragged(with: drag)
        XCTAssertGreaterThan(simd_distance(start, globe.camera.simdPosition), 0.1)
        XCTAssertEqual(simd_length(start), simd_length(globe.camera.simdPosition), accuracy: 0.0001)
        XCTAssertGreaterThan(simd_dot(globe.camera.simdWorldFront, -simd_normalize(globe.camera.simdPosition)), 0.9999)
    }
    func testLocationsRejectInvalidCoordinates() {
        XCTAssertFalse(GeoLocation(latitude: .nan, longitude: 10, country: "").isValid)
        XCTAssertFalse(GeoLocation(latitude: 91, longitude: 10, country: "").isValid)
    }
    func testLiveConnectionsEndpointMatchesDecoder() async throws {
        guard ProcessInfo.processInfo.environment["SMARTCLASH_LIVE_TEST"] == "1" else { throw XCTSkip("Opt-in read-only integration") }
        let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("SmartClash")
        let preferences = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("preferences.json"))) as! [String: Any]
        let snapshot = try await CoreClient(secret: preferences["secret"] as! String).connections()
        XCTAssertGreaterThan(snapshot.downloadTotal, 0)
        XCTAssertTrue(snapshot.connections.allSatisfy { !$0.id.isEmpty })
    }
    @MainActor
    func testLiveReadOnlyGlobeAndTrafficRender() async throws {
        guard ProcessInfo.processInfo.environment["SMARTCLASH_LIVE_RENDER"] == "1" else { throw XCTSkip("Opt-in read-only live rendering") }
        let root = Files.root
        let preferences = try JSONDecoder().decode(Preferences.self, from: Data(contentsOf: root.appendingPathComponent("preferences.json")))
        let nodes = SubscriptionService.cached(preferences.sources)
        let client = CoreClient(secret: preferences.secret)
        let connections = try await client.connections()
        let candidates = Array(connections.connections.filter { !$0.routeNodes(in: nodes).isEmpty && $0.metadata.geolocationKey != nil }.prefix(8))
        let candidate = try XCTUnwrap(candidates.first)
        let path = candidate.routeNodes(in: nodes)
        let geo = GeoIPService(cacheURL: nil)
        var locations: [String: GeoLocation] = [:]
        let hosts = candidates.flatMap { $0.routeNodes(in: nodes).compactMap { $0.payload["server"] as? String } + [$0.metadata.geolocationKey!] }
        for host in Set(hosts) {
            locations[host] = await geo.location(for: host)
        }
        let trace = ChainTrace.resolve(selected: path.last!.name, rawNodes: nodes, definitions: [])
        let snapshot = GlobeSnapshot.make(nodes: nodes, activeTrace: trace, connections: candidates, locations: locations, focusedConnection: nil)
        XCTAssertGreaterThan(snapshot.mappedConnections, 0)
        print("Live globe: \(snapshot.routes.count) routes, \(Set(locations.values).count) distinct locations")
        _ = NSApplication.shared
        let globe = GlobeScene(); globe.update(snapshot, animate: true)
        if let location = locations[path.first!.payload["server"] as! String] { globe.resetCamera(toward: location) }
        let renderer = SCNRenderer(device: nil, options: nil)
        renderer.scene = globe.scene; renderer.pointOfView = globe.camera
        let image = renderer.snapshot(atTime: 1, with: CGSize(width: 1400, height: 800), antialiasingMode: .multisampling4X)
        let bitmap = try XCTUnwrap(image.tiffRepresentation.flatMap(NSBitmapImageRep.init(data:)))
        let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try data.write(to: URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("dist/globe-live-render.png"))
    }
    @MainActor
    func testOfflineVisibleArcRender() throws {
        guard ProcessInfo.processInfo.environment["SMARTCLASH_LIVE_RENDER"] == "1" else { throw XCTSkip("Opt-in renderer verification") }
        _ = NSApplication.shared
        let tokyo = GeoLocation(latitude: 35.68, longitude: 139.76, country: "Japan")
        let sf = GeoLocation(latitude: 37.77, longitude: -122.42, country: "USA")
        var snapshot = GlobeSnapshot()
        snapshot.markers = [GlobeMarker(id: "node", name: "Tokyo", location: tokyo, isNode: true, active: true),
                            GlobeMarker(id: "target", name: "SF", location: sf, isNode: false, active: false)]
        snapshot.routes = [GlobeRoute(id: "test", locations: [tokyo, sf], selected: false, highlighted: true)]
        let globe = GlobeScene(); globe.update(snapshot, animate: false)
        globe.resetCamera(toward: GeoLocation(latitude: 30, longitude: 180, country: ""))
        let renderer = SCNRenderer(device: nil, options: nil); renderer.scene = globe.scene; renderer.pointOfView = globe.camera
        let image = renderer.snapshot(atTime: 1, with: CGSize(width: 1400, height: 800), antialiasingMode: .multisampling4X)
        let bitmap = try XCTUnwrap(image.tiffRepresentation.flatMap(NSBitmapImageRep.init(data:)))
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("dist/globe-fixture-render.png"))
        var orange = 0
        for y in stride(from: 0, to: bitmap.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: bitmap.pixelsWide, by: 2) {
                if let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB), color.redComponent > 0.7, color.greenComponent > 0.15, color.greenComponent < 0.8, color.blueComponent < 0.3 { orange += 1 }
            }
        }
        XCTAssertGreaterThan(orange, 50, "The rendered request arc must be visibly orange")
    }
}
