import XCTest
import SwiftUI
import AppKit
@testable import ProxyLogic
@testable import SmartClash

final class ChainPreviewTests: XCTestCase {
    @MainActor
    func testOfflineChainViewRenders() async throws {
        guard ProcessInfo.processInfo.environment["SMARTCLASH_RENDER_PREVIEW"] == "1" else {
            throw XCTSkip("Opt-in offline UI rendering")
        }
        let source = Source(name: "离线示例订阅", url: "https://example.com")
        let nodes = try SubscriptionParser.parse("""
        proxies:
          - {name: 入口节点, type: ss, server: entry.example.com, port: 443, cipher: aes-128-gcm, password: test}
          - {name: 出口节点, type: trojan, server: exit.example.com, port: 443, password: test}
        """, source: source)
        var exit = nodes[1].payload; exit["name"] = "AUTO-fixture"; exit["dialer-proxy"] = nodes[0].name
        let definition = ProxyNode(id: "fixture", name: "AUTO-fixture", sourceID: source.id, payload: exit)
        let store = AppStore(loadFromDisk: false)
        store.nodes = nodes; store.running = true
        store.preferences.sources = [source]; store.preferences.routingMode = .ai
        store.activeMode = .ai; store.routingStatus = "离线界面验收 · 示例数据"
        store.activeTrace = ChainTrace.resolve(selected: definition.name, rawNodes: nodes, definitions: [definition])
        for index in 0..<60 {
            let download = UInt64(300_000 + (sin(Double(index) / 4) + 1) * 500_000)
            let upload = UInt64(20_000 + (cos(Double(index) / 5) + 1) * 50_000)
            store.traffic.ingest(TrafficSample(up: upload, down: download, upTotal: UInt64(index) * 100_000,
                downTotal: UInt64(index) * 1_000_000), runID: "offline-fixture", at: Date().addingTimeInterval(Double(index - 59)))
        }
        let content = ChainView().environmentObject(store).padding(24).frame(width: 960, height: 1050)
            .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, .light)
        _ = NSApplication.shared
        let hosting = NSHostingView(rootView: content)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 960, height: 1050), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.contentView = nil; window.close() }
        hosting.frame = NSRect(x: 0, y: 0, width: 960, height: 1050)
        hosting.layoutSubtreeIfNeeded()
        try await Task.sleep(nanoseconds: 100_000_000)
        hosting.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
        hosting.cacheDisplay(in: hosting.bounds, to: bitmap)
        XCTAssertGreaterThan(bitmap.pixelsWide, 0)
        var colours = Set<String>()
        for y in stride(from: 0, to: bitmap.pixelsHigh, by: 20) {
            for x in stride(from: 0, to: bitmap.pixelsWide, by: 20) {
                if let colour = bitmap.colorAt(x: x, y: y) { colours.insert(colour.description) }
            }
        }
        XCTAssertGreaterThan(colours.count, 8, "Preview must contain rendered UI, not a blank container")
        let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        let filename = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("dist/chain-ui-preview-0.5.0.png")
        try data.write(to: filename, options: .atomic)
    }
}
