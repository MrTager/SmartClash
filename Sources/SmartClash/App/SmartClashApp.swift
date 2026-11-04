import SwiftUI
import AppKit
import ProxyLogic
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var store: AppStore?
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular); NSApp.activate(ignoringOtherApps: true)
    }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let store else { return .terminateNow }
        Task { sender.reply(toApplicationShouldTerminate: await store.shutdown()) }
        return .terminateLater
    }
}
@main
@MainActor
struct SmartClashApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @StateObject private var store = AppStore()
    var body: some Scene {
        WindowGroup("SmartClash", id: "main") {
            ContentView().environmentObject(store).onAppear { delegate.store = store }
        }
        .commands {
            CommandGroup(replacing: .newItem) { }
            CommandMenu("代理") {
                ForEach(TrafficMode.allCases) { mode in
                    Button("\(store.trafficMode == mode ? "✓ " : "")\(mode.title)") {
                        store.perform { try await store.changeTrafficMode(mode) }
                    }.disabled(store.recovery || store.stopping)
                }
                Divider()
                Button("切换系统代理") { store.perform { try await store.toggleSystem() } }
                    .keyboardShortcut("p", modifiers: [.command, .shift]).disabled(store.recovery)
                Button("停止并恢复网络") { store.requestStop() }
            }
        }
        MenuBarExtra("SmartClash", systemImage: store.running ? "network.badge.shield.half.filled" : "network") {
            Text(store.running ? "内核运行中 · \(store.trafficMode.title)" : "代理已停止")
            ForEach(TrafficMode.allCases) { mode in
                Button("\(store.trafficMode == mode ? "✓ " : "")\(mode.title)") {
                    store.perform { try await store.changeTrafficMode(mode) }
                }.disabled(store.recovery || store.stopping)
            }
            Divider()
            Button(store.systemProxy ? "关闭系统代理" : "开启系统代理") { store.perform { try await store.toggleSystem() } }.disabled(store.recovery)
            Button(store.tun ? "关闭 TUN" : "开启 TUN") { store.perform { try await store.toggleTun() } }.disabled(store.recovery)
            Button("停止并恢复网络") { store.requestStop() }
            Divider()
            OpenWindowButton()
            Button("退出") { NSApp.terminate(nil) }
        }
    }
}
@MainActor
private struct OpenWindowButton: View {
    @Environment(\.openWindow) var openWindow
    var body: some View { Button("显示主窗口") { openWindow(id: "main"); NSApp.activate(ignoringOtherApps: true) } }
}
