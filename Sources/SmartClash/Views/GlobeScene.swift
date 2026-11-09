import SwiftUI
import SceneKit
import AppKit
import simd
import ProxyLogic

@MainActor
final class GlobeScene: ObservableObject {
    let scene = SCNScene()
    let camera = SCNNode()
    private let content = SCNNode()
    private var previous = GlobeSnapshot()
    private var previousAnimation = true

    init() {
        scene.background.contents = NSColor(red: 0.025, green: 0.045, blue: 0.08, alpha: 1)
        camera.camera = SCNCamera(); camera.camera?.fieldOfView = 42
        camera.camera?.zNear = 0.1; camera.camera?.zFar = 30
        scene.rootNode.addChildNode(camera)
        let globe = SCNSphere(radius: 1); globe.segmentCount = 96
        let material = SCNMaterial(); material.lightingModel = .constant
        material.diffuse.contents = Self.earthTexture()
        globe.firstMaterial = material
        scene.rootNode.addChildNode(SCNNode(geometry: globe))
        scene.rootNode.addChildNode(content)
        addGrid(); resetCamera()
    }

    func resetCamera(toward location: GeoLocation? = nil) {
        let location = location ?? GeoLocation(latitude: 23, longitude: 110, country: "")
        camera.simdPosition = Self.position(location) * 3.9
        aimCamera()
    }

    func rotateCamera(horizontal: CGFloat, vertical: CGFloat) {
        let position = camera.simdPosition
        let distance = simd_length(position)
        let longitude = atan2(position.x, position.z) - Float(horizontal) * 0.008
        let latitudeLimit = Float.pi * 85 / 180
        let latitude = max(-latitudeLimit, min(latitudeLimit,
            asin(max(-1, min(1, position.y / distance))) + Float(vertical) * 0.008))
        camera.simdPosition = SIMD3(cos(latitude) * sin(longitude), sin(latitude), cos(latitude) * cos(longitude)) * distance
        aimCamera()
    }

    func zoomCamera(by amount: Float) {
        let position = camera.simdPosition
        let distance = simd_length(position)
        // Keep the unit sphere inside the 42-degree field of view, even after repeated scrolling.
        let nextDistance = max(3, min(8, distance * exp(-max(-4, min(4, amount)))))
        camera.simdPosition = position * (nextDistance / distance)
        aimCamera()
    }

    private func aimCamera() {
        camera.look(at: SCNVector3Zero, up: SCNVector3(0, 1, 0), localFront: SCNVector3(0, 0, -1))
    }

    func update(_ snapshot: GlobeSnapshot, animate: Bool) {
        guard previous != snapshot || animate != previousAnimation else { return }
        let selectionChanged = previous.selectedNodeID != snapshot.selectedNodeID || previous.selectedLocation != snapshot.selectedLocation
        previous = snapshot; previousAnimation = animate
        content.childNodes.forEach { $0.removeFromParentNode() }
        for marker in snapshot.markers {
            let sphere = SCNSphere(radius: marker.active ? 0.018 : 0.010)
            sphere.segmentCount = 10
            let color = marker.isNode ? (marker.active ? NSColor.cyan : NSColor.systemTeal) : NSColor.systemOrange
            sphere.firstMaterial = Self.material(color)
            let node = SCNNode(geometry: sphere); node.simdPosition = Self.position(marker.location) * 1.012
            node.name = "\(marker.name) · \(marker.location.label)"; content.addChildNode(node)
            if marker.active {
                let halo = SCNTorus(ringRadius: 0.032, pipeRadius: 0.002)
                halo.ringSegmentCount = 24; halo.pipeSegmentCount = 6; halo.firstMaterial = Self.material(color.withAlphaComponent(0.7))
                let ring = SCNNode(geometry: halo); ring.simdPosition = node.simdPosition
                ring.simdOrientation = simd_quatf(from: SIMD3<Float>(0, 1, 0), to: simd_normalize(node.simdPosition))
                content.addChildNode(ring)
            }
        }
        for route in snapshot.routes {
            let color = route.selected ? NSColor.cyan : NSColor.systemOrange
            let opacity: CGFloat = route.highlighted ? 0.95 : 0.20
            for index in 0..<(route.locations.count - 1) {
                let points = Self.arc(from: route.locations[index], to: route.locations[index + 1])
                let node = SCNNode(geometry: Self.tube(points: points, radius: route.selected ? 0.004 : 0.0025))
                node.geometry?.firstMaterial = Self.material(color.withAlphaComponent(opacity))
                content.addChildNode(node)
                if animate && route.highlighted {
                    let particle = SCNSphere(radius: 0.007); particle.segmentCount = 8
                    particle.firstMaterial = Self.material(color)
                    let tracer = SCNNode(geometry: particle)
                    let animation = CAKeyframeAnimation(keyPath: "position")
                    animation.values = points.map { NSValue(scnVector3: SCNVector3($0)) }
                    animation.duration = route.selected ? 3 : 2.4
                    animation.repeatCount = .infinity; animation.calculationMode = .linear
                    tracer.addAnimation(animation, forKey: "flow"); content.addChildNode(tracer)
                }
            }
        }
        if selectionChanged, let location = snapshot.selectedLocation, location.isValid { resetCamera(toward: location) }
    }

    private static func material(_ color: NSColor) -> SCNMaterial {
        let material = SCNMaterial(); material.lightingModel = .constant
        material.diffuse.contents = color; material.isDoubleSided = true
        return material
    }

    static func position(_ location: GeoLocation) -> SIMD3<Float> {
        let latitude = Float(location.latitude * .pi / 180), longitude = Float(location.longitude * .pi / 180)
        return SIMD3(cos(latitude) * sin(longitude), sin(latitude), cos(latitude) * cos(longitude))
    }

    static func arc(from: GeoLocation, to: GeoLocation) -> [SIMD3<Float>] {
        let a = position(from), b = position(to)
        let angle = acos(max(-1, min(1, simd_dot(a, b))))
        var axis = simd_cross(a, b)
        if simd_length(axis) < 0.0001 { axis = simd_cross(a, abs(a.y) < 0.9 ? SIMD3(0, 1, 0) : SIMD3(1, 0, 0)) }
        axis = simd_normalize(axis)
        return (0...56).map { index in
            let t = Float(index) / 56
            let direction = simd_quatf(angle: angle * t, axis: axis).act(a)
            return direction * (1.016 + sin(t * .pi) * min(0.38, 0.08 + angle * 0.14))
        }
    }

    private static func tube(points: [SIMD3<Float>], radius: Float) -> SCNGeometry {
        var vertices: [SCNVector3] = []; var indices: [UInt32] = []
        let sides = 6
        for i in points.indices {
            var tangent = points[min(i + 1, points.count - 1)] - points[max(0, i - 1)]
            if simd_length(tangent) < 0.00001 { tangent = simd_cross(points[i], abs(points[i].y) < 0.9 ? SIMD3(0, 1, 0) : SIMD3(1, 0, 0)) }
            tangent = simd_normalize(tangent)
            var normal = simd_cross(tangent, simd_normalize(points[i]))
            if simd_length(normal) < 0.00001 { normal = simd_cross(tangent, abs(tangent.y) < 0.9 ? SIMD3(0, 1, 0) : SIMD3(1, 0, 0)) }
            normal = simd_normalize(normal)
            let binormal = simd_normalize(simd_cross(tangent, normal))
            for side in 0..<sides {
                let angle = Float(side) * 2 * .pi / Float(sides)
                vertices.append(SCNVector3(points[i] + radius * (normal * cos(angle) + binormal * sin(angle))))
                if i > 0 {
                    let a = UInt32((i - 1) * sides + side), b = UInt32((i - 1) * sides + (side + 1) % sides)
                    let c = UInt32(i * sides + side), d = UInt32(i * sides + (side + 1) % sides)
                    indices += [a, b, c, b, d, c]
                }
            }
        }
        return SCNGeometry(sources: [SCNGeometrySource(vertices: vertices)], elements: [SCNGeometryElement(indices: indices, primitiveType: .triangles)])
    }

    private func addGrid() {
        var vertices: [SCNVector3] = []; var indices: [UInt32] = []
        func line(_ locations: [GeoLocation]) {
            let start = UInt32(vertices.count)
            vertices += locations.map { SCNVector3(Self.position($0) * 1.002) }
            for i in 1..<locations.count { indices += [start + UInt32(i - 1), start + UInt32(i)] }
        }
        for latitude in stride(from: -60, through: 60, by: 30) {
            line(stride(from: -180, through: 180, by: 3).map { GeoLocation(latitude: Double(latitude), longitude: Double($0), country: "") })
        }
        for longitude in stride(from: -180, to: 180, by: 30) {
            line(stride(from: -90, through: 90, by: 3).map { GeoLocation(latitude: Double($0), longitude: Double(longitude), country: "") })
        }
        let geometry = SCNGeometry(sources: [SCNGeometrySource(vertices: vertices)], elements: [SCNGeometryElement(indices: indices, primitiveType: .line)])
        geometry.firstMaterial = Self.material(NSColor.cyan.withAlphaComponent(0.10))
        scene.rootNode.addChildNode(SCNNode(geometry: geometry))
    }

    private static func earthTexture() -> NSImage {
        let resourceBundle = Bundle.main.resourceURL.flatMap {
            Bundle(url: $0.appendingPathComponent("SmartClash_SmartClash.bundle"))
        } ?? Bundle.module
        let size = NSSize(width: 2048, height: 1024)
        let image = NSImage(size: size)
        image.lockFocus()
        NSColor(red: 0.025, green: 0.075, blue: 0.13, alpha: 1).setFill()
        NSRect(origin: .zero, size: size).fill()
        if let url = resourceBundle.url(forResource: "EarthLand", withExtension: "geojson"),
           let data = try? Data(contentsOf: url),
           let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let features = json["features"] as? [[String: Any]] {
            for feature in features {
                guard let geometry = feature["geometry"] as? [String: Any] else { continue }
                let polygons: [[[[Double]]]]
                if geometry["type"] as? String == "Polygon", let rings = geometry["coordinates"] as? [[[Double]]] { polygons = [rings] }
                else if let rings = geometry["coordinates"] as? [[[[Double]]]] { polygons = rings }
                else { continue }
                for polygon in polygons {
                    let path = NSBezierPath(); path.windingRule = .evenOdd
                    for ring in polygon {
                        for (i, coordinate) in ring.enumerated() where coordinate.count >= 2 {
                            let point = NSPoint(x: (coordinate[0] + 180) / 360 * size.width, y: (coordinate[1] + 90) / 180 * size.height)
                            if i == 0 { path.move(to: point) } else { path.line(to: point) }
                        }
                        path.close()
                    }
                    NSColor(red: 0.10, green: 0.29, blue: 0.35, alpha: 1).setFill(); path.fill()
                    NSColor(red: 0.18, green: 0.51, blue: 0.56, alpha: 1).setStroke(); path.lineWidth = 0.6; path.stroke()
                }
            }
        }
        image.unlockFocus()
        return image
    }
}
