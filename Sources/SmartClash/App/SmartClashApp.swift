import SwiftUI
import AppKit
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
                Button("切换系统代理") { store.perform { try await store.toggleSystem() } }
                    .keyboardShortcut("p", modifiers: [.command, .shift]).disabled(store.busy || store.recovery)
                Button("停止并恢复网络") { store.perform { try await store.stop() } }.disabled(store.busy)
            }
        }
        MenuBarExtra("SmartClash", systemImage: store.running ? "network.badge.shield.half.filled" : "network") {
            Text(store.running ? "代理运行中" : "代理已停止")
            Button(store.systemProxy ? "关闭系统代理" : "开启系统代理") { store.perform { try await store.toggleSystem() } }.disabled(store.busy || store.recovery)
            Button(store.tun ? "关闭 TUN" : "开启 TUN") { store.perform { try await store.toggleTun() } }.disabled(store.busy || store.recovery)
            Button("停止并恢复网络") { store.perform { try await store.stop() } }.disabled(store.busy)
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
