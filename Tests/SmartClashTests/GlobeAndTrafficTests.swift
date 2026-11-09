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
    func testSwitchingExitUpdatesActiveMarkersAndFollowsNewNode() {
        _ = NSApplication.shared
        let a = ProxyNode(id: "A", name: "A", sourceID: UUID(), payload: ["server": "a.example.test"])
        let b = ProxyNode(id: "B", name: "B", sourceID: UUID(), payload: ["server": "b.example.test"])
        let nodes = [a, b]
        let aLocation = GeoLocation(latitude: 35, longitude: 139, country: "Japan")
        let bLocation = GeoLocation(latitude: 1, longitude: 103, country: "Singapore")
        let locations = ["a.example.test": aLocation, "b.example.test": bLocation]
        let oldConnection = CoreConnection(id: "old", metadata: ConnectionMetadata(destinationIP: "1.1.1.1"), chains: [a.name])
        let first = GlobeSnapshot.make(nodes: nodes, activeTrace: .resolve(selected: a.name, rawNodes: nodes, definitions: []),
            connections: [oldConnection], locations: locations, focusedConnection: nil)
        let globe = GlobeScene()
        globe.update(first, animate: false)
        XCTAssertEqual(first.selectedNodeID, a.id)
        XCTAssertEqual(first.selectedLocation, aLocation)
        XCTAssertEqual(first.markers.filter(\.active).map(\.name), [a.name])
        XCTAssertLessThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(aLocation) * 3.9), 0.0001)
        globe.rotateCamera(horizontal: 100, vertical: 20)

        let second = GlobeSnapshot.make(nodes: nodes, activeTrace: .resolve(selected: b.name, rawNodes: nodes, definitions: []),
            connections: [oldConnection], locations: locations, focusedConnection: nil)
        globe.update(second, animate: false)
        XCTAssertEqual(second.selectedNodeID, b.id)
        XCTAssertEqual(second.selectedLocation, bLocation)
        XCTAssertEqual(second.mappedNodes, 2)
        XCTAssertEqual(second.markers.filter(\.active).map(\.name), [b.name])
        XCTAssertEqual(second.markers.first(where: { $0.name == a.name })?.active, false)
        XCTAssertLessThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(bLocation) * 3.9), 0.0001)
    }
    @MainActor
    func testMultihopCameraFollowsLastHopInsteadOfFirstActiveMarker() {
        _ = NSApplication.shared
        let entry = ProxyNode(id: "entry", name: "Entry", sourceID: UUID(), payload: ["server": "a-entry.example.test"])
        let exit = ProxyNode(id: "exit", name: "Exit", sourceID: UUID(), payload: ["server": "z-exit.example.test", "dialer-proxy": entry.name])
        let nodes = [entry, exit]
        let entryLocation = GeoLocation(latitude: 35, longitude: 139, country: "Japan")
        let exitLocation = GeoLocation(latitude: 1, longitude: 103, country: "Singapore")
        let trace = ChainTrace.resolve(selected: exit.name, rawNodes: nodes, definitions: [])
        let snapshot = GlobeSnapshot.make(nodes: nodes, activeTrace: trace, connections: [],
            locations: ["a-entry.example.test": entryLocation, "z-exit.example.test": exitLocation], focusedConnection: nil)
        XCTAssertEqual(trace.hops.map(\.id), [entry.id, exit.id])
        XCTAssertEqual(snapshot.markers.first(where: { $0.active })?.name, entry.name)
        XCTAssertEqual(snapshot.selectedNodeID, exit.id)
        XCTAssertEqual(snapshot.selectedLocation, exitLocation)
        XCTAssertEqual(snapshot.routes.first?.locations, [entryLocation, exitLocation])
        let globe = GlobeScene()
        globe.update(snapshot, animate: false)
        XCTAssertLessThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(exitLocation) * 3.9), 0.0001)
    }
    @MainActor
    func testMissingExitLocationNeverFallsBackAndFollowsWhenItArrives() {
        _ = NSApplication.shared
        let a = ProxyNode(id: "A", name: "A", sourceID: UUID(), payload: ["server": "a.example.test"])
        let entry = ProxyNode(id: "entry", name: "Entry", sourceID: UUID(), payload: ["server": "entry.example.test"])
        let b = ProxyNode(id: "B", name: "B", sourceID: UUID(), payload: ["server": "b.example.test", "dialer-proxy": entry.name])
        let nodes = [a, entry, b]
        let aLocation = GeoLocation(latitude: 35, longitude: 139, country: "Japan")
        let entryLocation = GeoLocation(latitude: 1, longitude: 103, country: "Singapore")
        let bLocation = GeoLocation(latitude: 37, longitude: -122, country: "USA")
        var locations = ["a.example.test": aLocation, "entry.example.test": entryLocation]
        let globe = GlobeScene()
        globe.update(GlobeSnapshot.make(nodes: nodes, activeTrace: .resolve(selected: a.name, rawNodes: nodes, definitions: []),
            connections: [], locations: locations, focusedConnection: nil), animate: false)
        globe.rotateCamera(horizontal: 100, vertical: 30)
        let manualPosition = globe.camera.simdPosition
        let trace = ChainTrace.resolve(selected: b.name, rawNodes: nodes, definitions: [])
        let pending = GlobeSnapshot.make(nodes: nodes, activeTrace: trace, connections: [], locations: locations, focusedConnection: nil)
        XCTAssertEqual(pending.selectedNodeID, b.id)
        XCTAssertNil(pending.selectedLocation)
        XCTAssertEqual(pending.markers.first(where: { $0.name == a.name })?.active, false)
        XCTAssertEqual(pending.markers.first(where: { $0.name == entry.name })?.active, true)
        XCTAssertTrue(pending.routes.isEmpty)
        globe.update(pending, animate: false)
        XCTAssertEqual(globe.camera.simdPosition, manualPosition)
        XCTAssertGreaterThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(aLocation) * 3.9), 0.1)
        XCTAssertGreaterThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(entryLocation) * 3.9), 0.1)

        locations["b.example.test"] = bLocation
        let resolved = GlobeSnapshot.make(nodes: nodes, activeTrace: trace, connections: [], locations: locations, focusedConnection: nil)
        XCTAssertEqual(resolved.selectedNodeID, pending.selectedNodeID)
        XCTAssertEqual(resolved.selectedLocation, bLocation)
        globe.update(resolved, animate: false)
        XCTAssertLessThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(bLocation) * 3.9), 0.0001)
    }
    @MainActor
    func testSameExitPollingAndAnimationChangesPreserveManualCamera() {
        _ = NSApplication.shared
        let node = ProxyNode(id: "exit", name: "Exit", sourceID: UUID(), payload: ["server": "exit.example.test"])
        let trace = ChainTrace.resolve(selected: node.name, rawNodes: [node], definitions: [])
        let locations = ["exit.example.test": GeoLocation(latitude: 35, longitude: 139, country: "Japan"),
                         "1.1.1.1": GeoLocation(latitude: -27, longitude: 153, country: "Australia")]
        let first = GlobeSnapshot.make(nodes: [node], activeTrace: trace, connections: [], locations: locations, focusedConnection: nil)
        let globe = GlobeScene()
        globe.update(first, animate: false)
        globe.rotateCamera(horizontal: 140, vertical: 30)
        globe.zoomCamera(by: -0.2)
        let manualPosition = globe.camera.simdPosition
        let manualOrientation = globe.camera.simdOrientation
        for index in 0..<3 {
            let connection = CoreConnection(id: "request-\(index)", metadata: ConnectionMetadata(destinationIP: "1.1.1.1"), chains: [node.name])
            let polled = GlobeSnapshot.make(nodes: [node], activeTrace: trace, connections: [connection], locations: locations, focusedConnection: connection.id)
            XCTAssertNotEqual(polled, first)
            XCTAssertEqual(polled.selectedNodeID, first.selectedNodeID)
            XCTAssertEqual(polled.selectedLocation, first.selectedLocation)
            globe.update(polled, animate: true)
            globe.update(polled, animate: false)
            XCTAssertEqual(globe.camera.simdPosition, manualPosition)
            XCTAssertEqual(globe.camera.simdOrientation.vector, manualOrientation.vector)
        }
    }
    @MainActor
    func testSwitchingNodeIDsOnSharedServerStillFollowsExit() {
        _ = NSApplication.shared
        let a = ProxyNode(id: "A", name: "A", sourceID: UUID(), payload: ["server": "shared.example.test"])
        let b = ProxyNode(id: "B", name: "B", sourceID: UUID(), payload: ["server": "shared.example.test"])
        let nodes = [a, b]
        let location = GeoLocation(latitude: 35, longitude: 139, country: "Japan")
        let first = GlobeSnapshot.make(nodes: nodes, activeTrace: .resolve(selected: a.name, rawNodes: nodes, definitions: []),
            connections: [], locations: ["shared.example.test": location], focusedConnection: nil)
        let second = GlobeSnapshot.make(nodes: nodes, activeTrace: .resolve(selected: b.name, rawNodes: nodes, definitions: []),
            connections: [], locations: ["shared.example.test": location], focusedConnection: nil)
        XCTAssertEqual(first.selectedNodeID, a.id)
        XCTAssertEqual(second.selectedNodeID, b.id)
        XCTAssertEqual(first.selectedLocation, second.selectedLocation)
        XCTAssertEqual(second.markers.count, 1)
        XCTAssertEqual(second.markers.first?.name, b.name)
        XCTAssertEqual(second.markers.first?.active, true)
        let globe = GlobeScene()
        globe.update(first, animate: false)
        globe.rotateCamera(horizontal: 140, vertical: 30)
        XCTAssertGreaterThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(location) * 3.9), 0.1)
        globe.update(second, animate: false)
        XCTAssertLessThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(location) * 3.9), 0.0001)
    }
    @MainActor
    func testChangedLocationForSameExitRefocusesCamera() {
        _ = NSApplication.shared
        let node = ProxyNode(id: "exit", name: "Exit", sourceID: UUID(), payload: ["server": "exit.example.test"])
        let trace = ChainTrace.resolve(selected: node.name, rawNodes: [node], definitions: [])
        let original = GeoLocation(latitude: 35, longitude: 139, country: "Japan")
        let updated = GeoLocation(latitude: 1, longitude: 103, country: "Singapore")
        let first = GlobeSnapshot.make(nodes: [node], activeTrace: trace, connections: [], locations: ["exit.example.test": original], focusedConnection: nil)
        let second = GlobeSnapshot.make(nodes: [node], activeTrace: trace, connections: [], locations: ["exit.example.test": updated], focusedConnection: nil)
        XCTAssertEqual(first.selectedNodeID, second.selectedNodeID)
        let globe = GlobeScene()
        globe.update(first, animate: false)
        globe.rotateCamera(horizontal: 140, vertical: 30)
        globe.update(second, animate: false)
        XCTAssertLessThan(simd_distance(globe.camera.simdPosition, GlobeScene.position(updated) * 3.9), 0.0001)
    }
    @MainActor
    func testIncompleteUnknownOrInvalidExitDoesNotProvideFocusLocation() {
        _ = NSApplication.shared
        let entry = ProxyNode(id: "entry", name: "Entry", sourceID: UUID(), payload: ["server": "entry.example.test"])
        let exit = ProxyNode(id: "exit", name: "Exit", sourceID: UUID(), payload: ["server": "exit.example.test", "dialer-proxy": entry.name])
        let nodes = [entry, exit]
        let trace = ChainTrace.resolve(selected: exit.name, rawNodes: nodes, definitions: [])
        let location = GeoLocation(latitude: 35, longitude: 139, country: "Japan")
        let locations = ["entry.example.test": location, "exit.example.test": location]
        let incomplete = GlobeSnapshot.make(nodes: nodes, activeTrace: ChainTrace(hops: trace.hops, complete: false),
            connections: [], locations: locations, focusedConnection: nil)
        XCTAssertNil(incomplete.selectedNodeID)
        XCTAssertNil(incomplete.selectedLocation)
        let sameName = ProxyNode(id: "other-exit", name: exit.name, sourceID: exit.sourceID, payload: exit.payload)
        let unknown = GlobeSnapshot.make(nodes: [entry, sameName], activeTrace: trace, connections: [], locations: locations, focusedConnection: nil)
        XCTAssertNil(unknown.selectedNodeID)
        XCTAssertNil(unknown.selectedLocation)
        XCTAssertEqual(unknown.markers.first?.active, true)
        let invalidLocation = GeoLocation(latitude: 91, longitude: 139, country: "")
        let invalid = GlobeSnapshot.make(nodes: nodes, activeTrace: trace, connections: [],
            locations: ["entry.example.test": location, "exit.example.test": invalidLocation], focusedConnection: nil)
        XCTAssertEqual(invalid.selectedNodeID, exit.id)
        XCTAssertNil(invalid.selectedLocation)
        let globe = GlobeScene()
        globe.rotateCamera(horizontal: 140, vertical: 30)
        let manualPosition = globe.camera.simdPosition
        globe.update(incomplete, animate: false)
        globe.update(unknown, animate: false)
        var invalidFocus = GlobeSnapshot()
        invalidFocus.selectedNodeID = exit.id
        invalidFocus.selectedLocation = invalidLocation
        globe.update(invalidFocus, animate: false)
        XCTAssertEqual(globe.camera.simdPosition, manualPosition)
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
