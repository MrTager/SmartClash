import SwiftUI
import SceneKit

@MainActor
struct GlobeView: NSViewRepresentable {
    let renderer: GlobeScene

    func makeNSView(context: Context) -> GlobeSCNView {
        GlobeSCNView(renderer: renderer)
    }

    func updateNSView(_ view: GlobeSCNView, context: Context) { }
}

@MainActor
final class GlobeSCNView: SCNView {
    private let renderer: GlobeScene
    private var lastDragLocation: NSPoint?

    init(renderer: GlobeScene) {
        self.renderer = renderer
        super.init(frame: .zero, options: nil)
        scene = renderer.scene
        pointOfView = renderer.camera
        // SceneKit's default controls pan on scroll and switch to a separate camera.
        allowsCameraControl = false
        preferredFramesPerSecond = 30
        antialiasingMode = .multisampling2X
        isPlaying = true
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func mouseDown(with event: NSEvent) {
        lastDragLocation = event.locationInWindow
    }

    override func mouseDragged(with event: NSEvent) {
        let location = event.locationInWindow
        if let previous = lastDragLocation {
            renderer.rotateCamera(horizontal: location.x - previous.x, vertical: previous.y - location.y)
        }
        lastDragLocation = location
    }

    override func mouseUp(with event: NSEvent) {
        lastDragLocation = nil
    }

    override func scrollWheel(with event: NSEvent) {
        let sensitivity: Float = event.hasPreciseScrollingDeltas ? 0.01 : 0.08
        renderer.zoomCamera(by: Float(event.scrollingDeltaY) * sensitivity)
    }

    override func magnify(with event: NSEvent) {
        renderer.zoomCamera(by: Float(event.magnification))
    }
}
