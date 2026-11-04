import AppKit

@MainActor
enum AppArtwork {
    static let icon: NSImage? = Bundle.main.url(forResource: "AppIcon", withExtension: "icns").flatMap { NSImage(contentsOf: $0) }
}
