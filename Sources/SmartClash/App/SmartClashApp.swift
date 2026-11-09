import SwiftUI
import AppKit
import ProxyLogic
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var store: AppStore?
    private var terminationTask: Task<Void, Never>?
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular); NSApp.activate(ignoringOtherApps: true)
    }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard terminationTask == nil else { return .terminateLater }
        guard let store else {
            let alert = NSAlert()
            alert.messageText = "暂时无法安全退出"
            alert.informativeText = "生命周期状态尚未绑定，未确认网络恢复。请稍后重试退出。"
            alert.runModal()
            return .terminateCancel
        }
        terminationTask = Task {
            let stopped = await store.shutdown()
            sender.reply(toApplicationShouldTerminate: stopped)
            terminationTask = nil
            if !stopped {
                sender.activate(ignoringOtherApps: true)
                let alert = NSAlert()
                alert.alertStyle = .warning
                alert.messageText = "网络恢复未完成，已取消退出"
                alert.informativeText = store.error ?? "未确认后台资源已停止，请重试退出。"
                alert.addButton(withTitle: "重试退出")
                alert.addButton(withTitle: "留在应用中")
                if alert.runModal() == .alertFirstButtonReturn { sender.terminate(nil) }
            }
        }
        return .terminateLater
    }
}
@main
@MainActor
struct SmartClashApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @StateObject private var store = AppStore()
    private func bindDelegate() { delegate.store = store }
    var body: some Scene {
        // 在构造场景时绑定，不依赖首个窗口出现（菜单栏或早期退出也走清理）。
        let _ = bindDelegate()
        WindowGroup("SmartClash", id: "main") {
            ContentView().environmentObject(store)
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
